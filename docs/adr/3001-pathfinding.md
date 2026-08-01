# ADR 3001: Pathfinding — derived passability, occluders and passage, integer-cost grid A*

- Status: accepted and **implemented in full** — `cic_sim::ground` derives the grid, stamps it and
  searches it; `cic_sim::units` walks what it returns, re-plans when the ground under a route
  changes, and keeps units out of each other. Decision 6's string-pulling gained a corner-rounding
  pass this record did not describe, recorded as **amendment D** below rather than left as
  undocumented behaviour, and stamping exposed a defect in the same pass, recorded as **amendment
  E**. Decision 10 reserved ground rather than designing it, so what was decided while building it
  is written into that entry. **All five amendments below are accepted** — A, B and C by Denys on
  2026-08-01 together with [ADR 3002](3002-corridor-economy.md), which raised them, and D and E
  with the passes that introduced them. A, B, D and E are implemented; C has nothing to implement
  until combat produces a wreck.

## Context

[M6](../milestones/m6-gameplay.md) charters *pathfinding over the heightfield, with terrain
passability, dynamic obstruction from structures, and local avoidance between units*. The
command-to-motion pipe is proven: `cic_sim::units` moves a unit in a straight line, replay-identical,
built deliberately as the stepping stone to this. What does not exist is any simulation-side notion of
ground: units move on an unbounded plane, the terrain format carries heights and layer weights and
nothing about where a unit may stand, and templates have no footprint.

Constraints already in force and not this record's to revisit: [ADR
0007](0007-simulation-arithmetic.md)'s operation set is the only arithmetic that may touch simulation
state; the [determinism invariants](../invariants/determinism.md) forbid hashed containers wherever
order reaches state and require stable identifiers and per-subsystem hashing; and the M6 design note
already names pathfinding the subsystem most likely to break determinism, because the tempting
implementations use hashed containers and floating-point heuristics.

The [faction bible](../design/faction-bible.md) is upstream of this format, the same way it was
upstream of writable terrain textures. Three of its mechanics constrain the design directly:

- **Concord grades roads that speed movement.** Passability cannot be a bit; a cell needs a movement
  *cost*, and the costs must be editable at runtime, because paving the map is the faction's identity.
  The render side already treats route grading as a texture write; the simulation side needs the
  equivalent.
- **Meridian digs tunnels and holds crossings.** Some objects must make ground passable that the
  terrain says is not — a bridge over water, a tunnel through a cliff, a ramp over a ditch.
- **Structures stand on the map and deny the ground they stand on** — for every faction, and for
  Meridian's converted buildings especially, where whether to level a structure is a real decision.

Direction set by Denys, 2026-07-30: passability is **derived** from the terrain, not authored; objects
carry **occluder** and **passer** properties (a building occludes, a bridge grants passage); the
algorithm is **grid A\***; and the whole thing must not be over-engineered into a precision project —
facing in particular needs to be reproducible and natural-looking, not a six-decimal-place simulation.

## Decision

1. **Passability is derived from the heightfield, never authored.** At activation, each cell's base
   state is computed from data the package already carries: the slope between adjacent height samples
   against a threshold, and the water table (a cell whose ground sits below the water level is
   impassable). No terrain format change, no authored passability layer, no second source of truth that
   can disagree with the ground — which matters here more than in most engines, because this terrain
   *deforms*: heights are writable by design, and an authored layer would need re-authoring on every
   edit. The derivation uses comparisons and subtraction only, inside ADR 0007's set trivially.

2. **The pathfinding grid is the heightfield's own grid.** One pathfinding cell per sample interval —
   `(width − 1) × (height − 1)` cells at `horizontal_scale` spacing — so there is no second resolution
   to keep registered against the first, and a height edit maps to the cells it affects by index
   arithmetic. If a shipped map proves too fine to search, the recorded fallback is coarsening by an
   integer factor, not a free-floating second grid.

3. **A cell carries a small-integer cost class, not a bit.** `0` is impassable; `1` is plain ground;
   values below and above scale the cost of stepping through the cell. This is the faction bible
   arriving in the format: a graded road is a cell class cheaper than ground, mud and rubble are
   classes dearer, and the grading edit that Concord's construction will perform is a stamp of cell
   classes exactly as it is a texture write on the render side. One mover class for now — a cliff
   impassable to a truck is impassable to everyone — with per-class *interpretation* of the byte left
   open for when a mechanic wants infantry-only ground, rather than speculatively encoded now.

4. **Objects modify the grid through two template properties: a footprint that occludes, and a passage
   that grants.** Both are optional, both are rectangles of whole cells, and both arrive in
   `templates.json` with this mechanic, per that format's own rule that a field lands with its first
   consumer.
   - `footprint` — the cells the object occupies. While it stands, those cells are impassable,
     whatever the terrain says. A building is an occluder.
   - `passage` — the cells the object makes traversable, with a cost class. A bridge grants passage
     over water it spans; a tunnel grants passage through ground the slope test refused; a ramp grants
     passage at a grade. Passage *overrides the derivation*; elevation stays presentation's problem
     exactly as it is for units today — the simulation is a 2D ground plane and a unit on a bridge is
     a unit whose cells happen to be passable.
   - **Precedence: derivation, then passage, then occlusion.** Passage beats the terrain; a footprint
     beats everything, so a structure built at a bridgehead denies the bridgehead. An object's death
     or demolition removes its stamps and restores what the layers beneath it say.
   - **Placements snap to cells and footprints stamp in quarter turns.** A structure's position
     quantizes to the grid and its rectangle rotates in 90° steps; the *visual* rotation stays free.
     Stamping an arbitrarily rotated rectangle means rasterizing it deterministically, which is real
     work purchasing nothing a quarter-turn RTS footprint does not already deliver.

   **Implemented.** `templates.json` carries `footprint` and `passage`, both a rectangle in whole
   cells and the second with a class; only a `structure` or a `prop` may declare either, because a
   mover's own occupancy is decision 10's `radius` and a footprint that moved would be lifted and
   re-laid every tick. The grid keeps the derivation whole underneath and computes the classes the
   search reads from it plus the stamps, which is what makes "an object's death restores what the
   layers beneath it say" true rather than approximated — the test that matters puts one footprint
   over lake bed *and* open ground and requires each cell to come back as itself, because an
   implementation that wrote over the classes and restored plain ground gets two of three right and
   leaves a walkable lake. Two settlements the record left open: an even extent takes the extra cell
   on the high side of the cell its owner stands in, and a rectangle at the map edge is **clipped**
   rather than slid inward, so a building's footprint stays where its model is. Overlapping passages
   resolve to the cheapest class, which is order-independent so that which object was built first
   decides nothing.

   The one producer so far is scenario activation: `Ground` reads `Forces` as an earlier peer and
   reconciles once a tick, so construction, demolition, movement and Concord's grading all arrive
   through one comparison rather than through four call sites. A consequence worth stating because
   it surprised the first test written against it: **the stamps land on tick zero, not at
   registration** — a peer is only readable inside a tick.

5. **The algorithm is A\* on the 8-connected grid, in integer arithmetic throughout.** An orthogonal
   step costs `10 ×` the destination's cell class, a diagonal `14 ×` — the classic integer
   approximation of √2, chosen not for accuracy but because *consistency is the requirement and
   accuracy is not*: path cost is a ranking device, and integer costs mean the accumulated `g`, the
   octile-distance heuristic, and every comparison are exact on every machine by construction. There
   is no floating-point accumulation question to manage because there is no floating point in the
   search. This is the record's answer to over-engineering: the way to not spend effort controlling
   float error is to have none.
   - **Ties break on the lower cell index**, unconditionally. Equal-cost frontiers are common on a
     grid and an unspecified tie is exactly the nondeterminism the M6 design note warns about.
   - **The open set is a binary heap ordered on `(f, cell index)` over flat arrays indexed by cell.**
     No hashed container appears anywhere in the search.
   - **A diagonal step may not cut a corner** past an impassable cell: both orthogonal neighbours it
     brushes must be passable, or a unit's line would clip the building it is walking around.
   - **An unreachable target degrades to the nearest reachable cell** by heuristic distance among the
     cells the search closed — best effort toward the order, which is what a player asking for the far
     side of a wall means, and cheaper than a second search because the first one already did the work.

6. **A path is a waypoint list, string-pulled, walked by the stepper that already exists.** The A\*
   cell chain is smoothed by line-of-walkability checks so a unit crosses open ground in one straight
   run instead of staircasing, and the surviving waypoints feed the identical
   subtraction–`sqrt`–division step `cic_sim::units` performs today — the move verb's meaning changes
   from "walk straight at the target" to "walk the route", and its encoding does not change at all.

7. **Grid edits repath deterministically, on the tick they land.** Every stamp — construction,
   demolition, grading, a bridge lost — knows the cells it touched; a unit whose remaining waypoints
   cross a touched cell repaths that tick, in identifier order, and a unit whose next step is blocked
   repaths as the fallback for anything the intersection test misses. Worst case, one edit repaths
   many units inside one tick; that spike is accepted now and amortizing it (a deterministic repath
   queue spread over ticks) is recorded as the option if profiling objects.

   **Implemented, and the fallback turned out to be unnecessary.** `Ground` reports the rectangles
   it changed and clears them at the top of every tick, so an edit is news exactly once; `Units`
   runs after it, and every unit whose remaining route crosses one plans again from where it stands
   to where it was going. The intersection test is an **over-approximation** — a leg is tested by
   the bounding box of the cells its ends fall in, which contains every cell the leg actually
   crosses — so it can cost a repath that returns the same route and it cannot miss one. The
   "next step is blocked" clause above was written as a safety net for what the test misses; a test
   that cannot miss leaves it nothing to catch, and adding it anyway would have made a unit ordered
   into a cell that is impassable and its own re-search the same answer every tick for ever.

   A repath is **counted and hashed**, the same way an ignored command is: it changes where a unit
   goes with no order having been given, so two machines that repathed a different number of units
   diverge on the tick it happened rather than where the paths visibly part.

8. **The grid hashes incrementally.** A fingerprint of the derived grid folds into the subsystem hash
   at activation, and every stamp folds in as it is applied — because rehashing tens of millions of
   cells per tick is not a price any other subsystem pays, and the grid's state is a pure function of
   activation plus the stamps the command log already records. `first_divergence` therefore still
   names the tick a machine stamped differently.

   **Implemented as a chain**, which is what "folds in as it is applied" means and is worth stating
   plainly: the fingerprint records the *history* of edits and not only their sum, so lifting a
   stamp restores every cell and does not restore the number the grid had before it was laid. That
   catches strictly more than a state hash would, and nothing here needs to recognise a grid it has
   seen before.

9. **Facing stays out of simulation state until a mechanic reads it.** Presentation already derives a
   heading from motion, and with waypointed routes it keeps doing exactly that, smoothed with a
   turn-rate limit in presentation floats — reproducible because the motion it is derived from is
   deterministic, and natural-looking because that is presentation's whole license under ADR 0007
   decision 9. The day a mechanic *reads* facing — directional armour, turret arcs, formation
   orientation — it enters unit state as ADR 0007 decision 5's integer binary turns, one field, no new
   arithmetic machinery. Until then a simulated heading would be state nothing consumes, hashed and
   replayed for nobody.

10. **Local avoidance is a later slice, and deliberately modest.** Units today have no radius;
    `radius` joins the template set when avoidance lands, with its consumer. The intent of record is
    steering — push apart, slide along — not a reciprocal-velocity scheme; an RTS at this camera
    height needs units that do not stack, not crowd simulation. Ground reserved, not designed.

    **Implemented, and the ground reserved here is what the rest of this entry now uses.** A unit is
    a circle of `radius` metres; after every unit has stepped, each pair whose circles intersect
    gives up half the overlap along the line between their centres, scaled by one coefficient. That
    is "push apart". A push the grid refuses is retried one axis at a time, larger component first,
    so a unit shoved into a wall travels along it instead of stopping dead — that is "slide along".
    There is no negotiation between agents anywhere in it.

    Six things the record left open, decided here rather than in the code:

    - **`radius` is required for a `unit` and refused for everything else**, on the identical rule
      as `speed`. A standing object's occupancy is its `footprint` — whole cells, on the grid,
      never moving — and a mover's is a circle other movers push away from. The two are different
      mechanisms for the same idea and a template declaring the wrong one is a mistake worth
      refusing at load. The check is written once and used twice, because it is one rule.
    - **Every push is measured before any is applied.** Resolving in place would be deterministic —
      the roster is keyed by identifier — but it would give the lower identifier right of way, and
      "the unit built first wins the shoving match" is a rule somebody would have to explain later.
      Two passes cost one array of positions a tick and mean identifier order decides nothing.
    - **Two units in exactly the same place have no line to push along**, so that tie *is* broken on
      identifier order, along X. It is the only thing order decides here.
    - **The clamp is about the unit's centre, not its circle.** Routes run through cell centres and
      the stepper has always been a point on the grid; inflating obstacles by a radius is a real
      design and this decision does not ask for one.
    - **One coefficient**, `AvoidanceRules::separation` — how much of an overlap comes out of it per
      tick — folded into the subsystem hash for the reason `GroundRules` folds its own in. `0.0` is
      how "no avoidance" is spelled, rather than a second flag that could disagree with it.
    - **A crowd is measured against itself.** Sixteen units ordered to one cell on the rough test
      map end with 17 of 120 pairs overlapping, against **120 of 120** with the coefficient at
      zero; on the tightest muster point the map has, nobody is ever pushed onto ground the grid
      refuses.

    **Two limitations, recorded rather than hidden.** Units walking directly into each other stall,
    each pushing the other back as fast as it walks forward, because a head-on push has no sideways
    component to slide on; fixing that means choosing a side, which means a rule about *which*
    side, which is the negotiation this decision declined. And a crowd ordered to one point keeps
    jostling rather than settling — the 17 pairs above are that, not a failure to converge — because
    every unit is still walking at a point every other unit is standing on. The fix for the second
    is **formation movement**, an [M6](../milestones/m6-gameplay.md) charter line in its own right:
    sixteen units sent to one place should be given sixteen places.

    **The quadratic is not the problem it looks like**, and is left alone on measurement rather than
    on faith. One tick of `cic_sim::units` over units standing still — which is the pair loop plus a
    stepper with nothing to do — costs **0.013 ms at 100 units, 0.14 ms at 500 and 0.49 ms at
    1000**, against a 33 ms tick. A uniform spatial bin over the same cells the grid already has is
    the recorded mitigation, in the same spirit as decision 5's coarsening: written down so it is
    not reinvented, and not built until something measures a reason.

## Rationale

**Derived passability, against an authored layer.** An authored layer was the credible alternative —
it gives a designer direct control and it is how many editors work. It loses here because this
engine's terrain is writable at runtime by design (Concord paves; craters will come), so authored
passability either goes stale on the first edit or needs a re-derivation step anyway — at which point
the derivation *is* the system and the authored layer is a cache of it. Where a designer needs to
override the ground, the override is an *object* with `passage` or `footprint`, which is visible,
selectable, and destroyable — a better authoring unit than an invisible paint layer, and the only kind
of exception the factions actually need.

**A grid, against a navmesh.** A navmesh wins on open-field path quality and loses on everything this
project cares about: dynamic obstruction means online mesh edits, which is the hard half of navmesh
research; deterministic mesh generation is subtle enough to be its own project; and the input here is
*already a grid* — the heightfield — so the mesh would be derived from cells only to approximate
cells. A grid stamps a footprint in O(footprint) and never rebuilds. The path-quality gap closes to
acceptable with string-pulling, per decision 6.

**A\* per order, against flow fields.** Flow fields pay off when many units share one destination
every tick; they cost a field per destination and this game's orders are not yet shaped that way. A\*
is the simplest thing that is correct, and decision 2's fallback (integer coarsening) plus a
hierarchical layer are both compatible retrofits if profiling on real maps demands them. Deferred,
not rejected.

**Integer costs, against `f64` costs.** ADR 0007's `f64` would be *legal* here — addition and
comparison are correctly rounded — but legality is not the bar. A search accumulates thousands of
additions along thousands of frontier paths, every accumulation order a potential puzzle for whoever
next investigates a desync, and all of it buying a cost precision that path *ranking* cannot even
observe. Integers make the entire class of question unaskable. This is the same shape as ADR 0007
decision 5: choose the representation whose failure modes do not exist.

**Quarter-turn footprints, against free rotation.** Rasterizing a rotated rectangle onto cells
deterministically is solvable — and solving it would deliver footprints at 37° for a genre whose
buildings have snapped to grids since the genre existed. The visual transform stays free, so nothing
on screen is constrained by this; only the *stamp* is quantized.

## Consequences

- `templates.json` gains `footprint`, `passage` (with a cost class), and later `radius` — each
  arriving with its consumer, per that format's growth rule. Structures acquire their first
  simulation-side meaning. All three have landed, `radius` with decision 10's separation pass.
- The scenario/package layer must place structures on cell boundaries or accept quantization at
  activation. It accepts the quantization: a placement's position picks the cell it falls in and the
  rectangle is centred on that, so no authoring rule changed and nothing in the scenario format
  needed a new field.
- A new `cic-sim` subsystem owns the grid, its edits, and pathing requests; `units` consumes routes
  instead of raw targets. The move verb's payload is unchanged.
- Water level must be visible to the simulation. Today the viewer derives a water table
  presentation-side; the derivation (or the authored level) moves to package data the kernel can
  read, or water cannot block movement deterministically. This is the one place the decision reaches
  a format beyond templates.
- The repath-on-edit spike (decision 7) and search cost on very large maps (format ceiling 8192²,
  typical maps far smaller) are accepted risks with named mitigations: a deterministic repath queue,
  integer grid coarsening, and a hierarchical layer — in that order, none built until measurement
  asks.
- Elevation-as-presentation now has a visible seam: a unit on a bridge is drawn at deck height by
  presentation while the simulation walks the 2D plane. Fine until a mechanic cares about altitude
  (line of sight over a gorge); when one does, that mechanic's record revisits it.
- Air movers are outside this record entirely. AEC's aircraft, when they exist, do not consult the
  ground grid; their record owns what they consult instead.
- The AI opponent (M6's harness, M7-adjacent) inherits a queryable, deterministic map representation
  for free — reachability and chokepoints are grid questions. Its decision layer is expected to be
  data-driven (behaviour trees authored per faction rather than hard-coded), which is out of scope
  here and noted so the grid's query surface is designed as *the* consumer-facing API rather than
  `units`-private plumbing.

## Amendments A, B and C — accepted, raised by ADR 3002

All three came out of writing [the corridor economy](3002-corridor-economy.md) against this record
rather than out of implementing anything. The first two are defects; the third settles a choice
decision 4 deliberately left open. None of them reverses a decision — they finish one.

**Accepted by Denys on 2026-08-01, together with the record that raised them.** A and B are also
arrived at independently by the [faction bible](../design/faction-bible.md), which gives Concord
"graded roads that speed movement" — so they were obligations of accepted content whatever became of
ADR 3002, and that is the reason they were not left to wait on it.

### A. Plain ground is not cost class `1`

**The defect.** Decision 3 says a graded road is "a cell class cheaper than ground" and sets `0` to
impassable and `1` to plain ground; decision 5 sets an orthogonal step at `10 ×` the class. With
integer classes and `0` reserved, **there is no value cheaper than plain ground.** As written, grading
can restore mud and never improve past it — which turns Concord's rising income curve, the arithmetic
its entire doctrine rests on, into pothole repair.

**The fix, which changes no format and no arithmetic:** plain ground stops being `1`. Metalled road
`1`, graded road `2`, plain ground `3`, mud `4`, rubble `5` and up. `0` stays impassable, `10`/`14`
stays the octile pair, and the only thing that moves is which class number means which ground.

A metalled road at 3× open ground is aggressive on purpose. A road that is 20% better than a field is
not a thing three factions go to war over, and this one is the premise.

**The rejected alternative** is a lookup table from class to cost, which buys finer gradations —
road 7, ground 10, mud 16 — at the price of a table to author, version and keep deterministic. Not
worth it until the coarse ladder above proves too coarse, and recorded so it is not reinvented from
scratch when it does.

**Implemented.** The ladder is named in `cic_sim::ground` — `METALLED`, `GRADED`, `PLAIN`, `MUD`,
`RUBBLE` — and `GroundRules::plain_class` starts at `PLAIN`. It cost one default value, because the
heuristic had already been made to price itself against the cheapest class the grid holds rather than
against a hardcoded `1`; `a_renumbered_cost_ladder_finds_the_same_route` walks one route under three
ladders and requires the same answer, which is what makes the renumbering provably free.

### B. A cell's cost class must reach the movement rate, not only path ranking

This record is about *search*: the class scales the cost of a step and therefore which route wins.
Nothing in it makes a unit physically travel faster on a good road — `units` moves in a straight line
at the template's `speed` and consults no grid.

For Concord's paving to be an income increase rather than a routing preference, the same class has to
multiply movement rate. Stated here because the gap sits exactly on the seam between two subsystems,
which is where it would otherwise be found by wondering why grading the whole map changed nothing.

The arithmetic stays inside [ADR 0007](0007-simulation-arithmetic.md): a ratio of two small
integers against a per-tick displacement, correctly rounded, no transcendental and no new
representation.

**Implemented**, with one thing this amendment did not say. A ratio needs a denominator: something
has to declare which rung a template's authored `speed` is the speed *for*, or "three times faster"
has no referent. That is `GroundRules::reference_class`, defaulting to `PLAIN` and kept separate from
`plain_class` because they answer different questions — what the terrain derives to, against what a
speed means. A map that paved its open ground would move the first and leave the second alone.

The pace is sampled once per tick from the cell the unit stands in, not per leg. At thirty ticks a
second a unit spends tens of ticks inside one eight-metre cell, so the difference is unobservable and
the simpler rule is the one that can be reasoned about later.

### C. Wrecks stamp a cost class, not a footprint

Not a defect — a choice this record's decision 4 leaves open, settled here because the economy depends
on it. A wreck is an object and could stamp either. It stamps a **cost class**, because a footprint is
impassable and one dead truck is something a column pushes past rather than a wall.

The consequence is that a road closes by *accumulation* rather than by a single loss, which is the
behaviour worth having: a sustained battle chokes the corridor with its own casualties, and the
choking outlives the shooting. Ordinary decay clears it, and Meridian recovering the wrecks clears it
faster — so the faction that lives on throughput is paid to restore it.

**Nothing to implement, but the mechanism it needs now exists.** There is no combat, so there are no
wrecks. Decision 4 is built, so a wreck's cost class is a `passage` with a dear one — rubble at `5`
rather than a road at `1` — laid and lifted through the same reconcile everything else uses.
Accepted now so that whoever writes the first wreck does not have to decide it again, and does not
reach for a footprint because a footprint is the easier thing to reach for.

One question this leaves for that record rather than answering here: `passage` **overrides the
derivation**, so a wreck stamped on ground the terrain called impassable would make it crossable.
That is right for a bridge and wrong for a burnt-out truck in a river, and the difference is a
mechanic's to settle, not this one's.

## Amendments — implemented, each raised by building the one before it

### D. A route's corners are rounded, and the rounding is checked against the grid

**What the record missed.** Decision 6 says a path is "a waypoint list, string-pulled, walked by the
stepper that already exists", and that is what was built. It is also not enough. String-pulling
removes the staircase along a straight run but leaves every surviving corner at a cell centre, so a
unit crosses open ground beautifully and then turns 45° on the spot at an eight-metre interval. The
first person to watch it called the movement clunky, which is the correct word: the route was
optimal and the walking was clockwork. No test could have reported this — every assertion about
routes was about where they go, and this is about how they are taken.

**The fix.** Each interior corner is cut back by a radius and interpolated across a short quadratic
Bézier, the original corner serving as the control point. Both the radius and the number of segments
are coefficients on `GroundRules`, so a map or a mover class can want a different one, and `0.0`
restores the sharp behaviour exactly.

**The constraint that makes it safe, and it is the whole of the amendment.** Decision 5 forbids a
diagonal step cutting a corner past an impassable cell, and a smoothing pass that ignored the grid
would put that cut straight back — worse, it would put it back *precisely at the turns a unit makes
to walk around something*, which is where it matters. So every segment of a rounded corner is
checked against the grid, and a corner whose arc would clip anything stays sharp. Rounding may
therefore be refused, silently and per corner; a route is never smoothed into an obstacle.

**Why this sits in the simulation rather than in presentation.** It changes where the unit actually
is, so it is state, so it is hashed. Only the *facing* is presentation's, which decision 9 already
says and which the viewer now implements with the turn-rate limit that decision specifies. The two
together are what stop a unit reading as a piece on a board: the position curves, and the model
turns into the curve instead of snapping at each waypoint.

**The other half of the same complaint was not pathfinding at all.** Presentation was drawing raw
thirty-hertz snapshot positions at whatever rate the window ran, so units stepped. That is fixed by
interpolating between the last two ticks — `TickAccumulator::alpha`, which had existed since M5
documented as "what a renderer interpolates by" with nothing calling it. Worth recording here
because the symptom was indistinguishable from a pathfinding fault and the cause was in a different
crate.

### E. String-pulling was cost-blind, and the first cheaper-than-ground cell is what showed it

**The defect.** Decision 6 says a route is "string-pulled", and decision 5 says a shortcut may not
cut a corner past an impassable cell — so the smoothing pass asked one question of each candidate
line: *can a unit walk it?* That is the right question on a grid where every passable cell costs the
same, and it is the wrong one the moment they do not. A route that goes four rows out of its way to
reach a metalled road is **shorter in cost and longer in metres**, so a walkability-only pass pulls
it straight back off the road — A\* makes the decision the whole cost ladder exists for and the next
pass silently undoes it.

Nothing could have caught this before now, and that is the point of recording it rather than fixing
it quietly. `GroundRules::plain_class` could already be renumbered, but only for the *whole map*, so
no route ever crossed a cost gradient; decision 4's `passage` is the first thing in the engine that
puts one cell next to a cheaper one. Amendments A and B were about the ladder meaning something, and
this is the third place it had to reach.

**The fix, which changes no format and adds no arithmetic.** A waypoint is dropped when the straight
line is walkable **and costs no more than the chain it replaces**. The line's cells are already
being walked to check they are passable, so they are priced on the same walk, at the search's own
`10`/`14` against the entered cell's class; the chain's own cost comes from a prefix sum over the
cells A\* closed. Integers throughout, comparable with an accumulated `g` by construction.

**Why this is not a behaviour change anywhere else.** On ground of one class the two costs are
always *equal* — a Bresenham line between two cells takes `min(dx, dy)` diagonal steps and
`|dx − dy|` orthogonal ones, which is the octile-optimal mix, which is what A\* found — so every
shortcut that used to be taken is still taken. A\* returns a minimum-cost chain, so a straight line
can never come in under it; the only shortcuts now refused are the ones that would have cost the
unit something. `a_route_across_open_ground_is_one_straight_run` did not move.

**Corner rounding is deliberately left cost-blind.** Amendment D's arcs are checked for
walkability only. Rounding perturbs a route by a few metres at a corner, it exists to make walking
look like walking, and pricing it would buy nothing a player could see.
