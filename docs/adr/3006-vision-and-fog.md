# ADR 3006: Vision, fog and detection — per-group sight on the ground grid, remembered objects, a fourth axis

- Status: proposed

## Context

[M6](../milestones/m6-gameplay.md) charters *fog of war and shroud, per player, with the visibility
state living in the simulation*, and its design notes say why the placement is not negotiable: what a
player can see determines what their units may target, so vision is a gameplay rule the renderer
merely visualises, and getting it backwards makes vision-based orders non-deterministic.
[mechanics.md §5](../design/mechanics.md#5-vision-fog-and-detection) gives the model in prose — three
states per object per player, remembered state allowed to be wrong, detection as a fourth axis rather
than a fourth state, an object that reveals itself by firing — and no mechanics. Issue #76 item 4
names the gap. [ADR 3001](3001-pathfinding.md)'s Consequences explicitly deferred "line of sight over
a gorge" to "that mechanic's record"; this is that record.

Two proposed records already lean on it. [ADR 3004](3004-ai-command-seam.md) decision 2 requires that
when fog lands the AI read *its seat's visibility view* exactly as the interface does — this record
supplies that surface. [ADR 3005](3005-route-graph.md) established the pattern this subsystem follows:
registered after `Ground`, reading earlier peers, hashed to be attributable.

The factions make this axis their identity, so the design documents constrain the shape.
[faction-mechanics.md §4](../design/faction-mechanics.md#4-aec--allied-expeditionary-command) gives
AEC *purchased vision* — ISR sweeps and satellite windows that reveal an area for a duration — and
[§6](../design/faction-mechanics.md#6-meridian--meridian-council) gives Meridian *purchased
ambiguity*: concealment as its defence, an army that "collapses once observed", and a permit desk
that becomes visible to the payer once it assesses its first load
([mechanics.md §2.5](../design/mechanics.md#25-the-three-acquisitions)) — a targeted reveal this
record's model must be able to express. [balance.md §3.3](../design/balance.md#33-modifiers) already
prices vision against a reference value of 140 (metres, against ADR 3001's eight-metre cells), and
[§4.2](../design/balance.md#42-the-asymmetry-budget) names sight a paid asymmetry dimension — so
`vision` is a per-template number, which [templates.md](../formats/templates.md) already promises
under its growth rule: a field arrives with its first consumer, and this record is the consumer.

Constraints in force and not revisited here: [ADR 0007](0007-simulation-arithmetic.md)'s operation
set, and the [determinism invariants](../invariants/determinism.md) — ordered containers wherever
order reaches state, per-subsystem hashing, peers read under the registration-order contract.

## Decision

1. **Visibility is a `cic-sim` subsystem, computed on the ground grid's own cells.** `Vision`
   registers after `Ground` and after the movers it watches, so a tick's reads see this tick's final
   positions: it reads `Forces` for placed objects, `Units` for movers, and `Ground` for the grid's
   dimensions and cell size, all as earlier peers, exactly as `Ground` reads `Forces`. It computes on
   one cell per heightfield sample interval — the resolution ADR 3001 decision 2 already registered —
   because a second grid would be a second registration to keep aligned, which is the same argument
   that record made against a free-floating pathfinding resolution. If a shipped map proves too large,
   the recorded fallback is the same as ADR 3001's: coarsening by an integer factor of the ground
   grid, mapped by index arithmetic, never a second free resolution.

2. **The three states are two bits of truth per cell.** Per vision group (decision 7), per cell:
   **ever-seen**, which is monotone and never clears, and **observed-now**, which is recomputed as
   sources change. *Shrouded* is not-ever-seen — terrain unknown. *Observed* is observed-now.
   *Fogged* is ever-seen and not observed-now — terrain known, objects remembered per decision 5.
   The states are derived from the bits rather than stored as an enum, so no transition table exists
   to get wrong: flipping ever-seen on first observation is the only write the state machine has.

3. **Vision sources are template-declared, and the update is change-driven over per-cell counters.**
   Any `unit` or `structure` whose template declares `vision` (metres — Concord's observation posts
   see; a prop does not) contributes a disc of cells: radius in cells is the metres value divided by
   the grid's cell size, **truncated** — truncation is already this project's specified integer
   behaviour ([balance.md §2](../design/balance.md#2-arithmetic)), and rounding down means an
   authored figure is a ceiling the simulation never exceeds. A cell is inside the disc when the
   squared centre-to-centre cell distance is at most the squared radius — integer comparison, no
   `sqrt`, inside ADR 0007's set trivially. Every tick, each source whose cell changed — spawned,
   died, or moved *across a cell boundary*; movement inside one cell is not a change — decrements a
   per-cell, per-group counter over its old disc and increments it over the new one. Observed-now is
   counter-above-zero, and a counter crossing zero upward sets ever-seen. This is ADR 3001
   decision 7's replan-on-change shape applied to sight: a tick where nothing crossed a cell boundary
   costs nothing, and overlapping sources need no rescan because addition commutes — the counter is
   what makes source order decide nothing.

4. **Vision is radial. Elevation does not block line of sight.** The heights *are* in the kernel —
   ADR 3001 derives passability from them — so terrain-occluded sight is computable deterministically;
   this is a decision about cost and legibility, not feasibility. A unit behind a ridge is hidden by
   **concealment** — a mechanic, purchased, priced — not by geometry. Elevation stays a presentation
   fact, exactly where ADR 3001's Consequences left it, and this record keeps it there deliberately,
   for the reason that record's decision 10 kept avoidance modest: the corridor game's information
   war is authored into the factions — sweeps against camouflage, desks against detection — and a
   geometric occlusion layer would sit *under* that war, expensive, and illegible from across the
   room. **The reopen condition, recorded:** the day a mechanic wants a scout on a hill to matter —
   spotting for artillery, an overlook bonus — this is the paragraph to reopen, and the start is
   integer ray sampling over the heightfield along the same Bresenham cell walk `Ground` already owns
   (amendment E's line pricing walks it today). Until then, occlusion would be state nothing consumes.

5. **A player's memory of fogged objects is kernel state, per group, hashed.** When an object stops
   being observed by a group, the group records it: **template, owner, and position as last
   observed**, keyed by object identifier in an ordered map. A remembered object is allowed to be
   wrong — destroyed since, moved since — and is corrected only by observation: on re-observing the
   object, its memory entry is removed (it is live again, and will be re-recorded when it next
   fades); on observing the cell its memory claims while the object is not there, the entry is
   cleared. An object that dies *while observed* leaves no memory — the group watched it go. Memory
   is kernel state because its consumers demand determinism: the AI reads its seat's view
   (ADR 3004 decision 2), a saved match must restore what each player remembered, and mechanics.md
   §5 makes the wrongness itself a feature — a fogged structure that is already rubble still appears,
   which is where a feint pays.

6. **Detection is the fourth axis, represented as cover plus reveals.** A template may declare
   `concealed`; a concealed object standing inside a group's vision is **not observed** until
   detected. A template may declare `detection` (metres) — a second disc, maintained by the identical
   counter machinery as decision 3, one more counter plane per group. A concealed object is observed
   by a group when its cell is inside both the group's vision *and* its detection. Beside the discs,
   an object carries **reveal stamps**: `(group, until_tick)` entries making the object observed by
   that group until the tick passes, regardless of vision or concealment. *This record defines the
   field; combat defines when firing writes it* — "reveals itself by firing" is combat stamping every
   group, for a duration that is combat's number. Meridian's desk serving notice is a reveal stamped
   for one group: the payer. Both `concealed` and `detection` arrive in `templates.json` with their
   first declaring content — the axis is decided here, the fields land with Meridian's kit and the
   first detector, per the growth rule.

7. **Vision is shared within a team, and storage is per vision group.** The scenario already seats
   teams (`players[].team`); allies who could not share sight would re-litigate every co-op mechanic
   later, and deciding it later re-plumbs the storage. A **vision group** is: the players sharing a
   non-zero team, or a single player whose team is `0` (unallied, per
   [scenario.md](../formats/scenario.md)). The seat→group mapping is fixed at activation, in seat
   order. Counters, bits, and memory are all per group — one grid set for a whole team rather than a
   per-player set unioned at query time, because the union would be paid on every read while the
   group grid is paid once on every write, and because per-player memory inside one team would let
   two allies remember different worlds from the same shared observation, which nobody could explain.
   The cost is that mid-match alliance changes are not expressible; teams are authored, and the day
   diplomacy exists its record reopens this.

8. **What reads it.** Targeting, when combat lands, may select only objects the shooter's group
   observes — mechanics.md §5's premise and the reason this is simulation state at all
   ([§3.1](../design/mechanics.md#31-the-model)'s resolution begins from a target legally acquired).
   The interface renders its seat's view: ever-seen for the shroud, observed-now for fog, memory for
   ghosted objects. The AI reads the same per-seat snapshot surface, which is ADR 3004 decision 2
   satisfied — an AI that downcasts past it is cheating in the ordinary sense, not desyncing.
   Scripts are out of scope: [ADR 7002](7002-script-events.md)'s zones and host verbs are where a
   mission would query or grant sight, and that is its family's record to extend.

9. **Hashing follows ADR 3001 decision 8's incremental pattern, applied to everything that decides.**
   The grids are millions of cells across groups; rehashing them whole every tick is the price that
   record already declined to pay. So the subsystem keeps a chained fingerprint and folds in every
   write as it is applied: each counter delta, each ever-seen flip, each memory write and clear, each
   reveal stamp. The alternative — hashing nothing and letting divergent vision surface through its
   consumers, since divergent vision diverges combat anyway — was considered and rejected: it is
   true, and it is exactly what per-subsystem hashing exists to prevent, because `first_divergence`
   would then name combat three ticks after vision drifted and the hunt would start in the wrong
   subsystem. The house style is hash-what-decides, and observed-now decides targeting, so it is in.
   The seat→group mapping folds in at activation, like the grid fingerprint — two machines that
   grouped seats differently are playing different games and should diverge on tick zero, attributed.

10. **What this record does not decide.** Vision and detection radii values, and the concealment
    price — balance's ([§3.3](../design/balance.md#33-modifiers) already carries provisional rows).
    When firing reveals, to whom, and for how long — combat's record writes the stamps this record
    defines. Scripted reveals as host verbs — ADR 7002's family. The renderer's shroud presentation,
    the minimap, and ghost styling — presentation. AEC's ISR sweep mechanics, cooldown economy, and
    Meridian's false signatures — faction work standing on this substrate; a sweep is a temporary
    vision source and a false signature is a memory write, and both are expressible without touching
    this record.

## Rationale

**Why a cell grid rather than per-object visibility tests.** The credible alternative is no grid at
all: an object is observed when a range test against every enemy source says so, recomputed or cached
per pair. It loses on three counts. Shroud is about *terrain*, not objects — "never seen" needs a
per-cell fact, so some grid exists regardless and the question is only whether objects use it too.
Pair tests are O(sources × objects) with no way to amortise a stationary garrison, where the counter
grid makes a tick with no cell crossings free. And the renderer needs cells anyway; one
representation serving targeting, memory, AI and shroud is the same argument that put pathfinding on
the heightfield's own grid.

**Why counters rather than a bounded recompute.** The honest alternative to decision 3 is simpler
state: no counters, just recompute observed-now over the neighbourhood of every changed source. Its
defect is that a cell leaving one source's disc may still be covered by another, *unchanged* source,
so a correct recompute must find every source whose disc overlaps the neighbourhood — a spatial query
over sources, per change, that the counter makes unnecessary by construction. The counter costs two
bytes per cell per group and pays it back every tick: decrement and increment are unconditional,
commutative, and cannot miss. A `u16` saturating counter is the representation; sixty-five thousand
overlapping sources on one cell is a map this genre does not ship.

**Why radial vision rather than sampled-ray occlusion.** Argued in full because feasibility is not
the question. Rays are richer: a ridge is cover, an overlook is worth taking, terrain reads as
tactically alive. They cost per source per cell per change — each disc update becomes a bundle of
heightfield walks rather than a counter sweep — and they couple this subsystem's state to terrain
edits: heights are writable by design, a demolished building or a fresh crater changes sightlines, so
every stamp would fan out into vision recomputes and the vision hash would move when the *ground*
moved, exactly the cross-subsystem attribution smear ADR 3005 rejected when it refused to derive link
geometry from the grid. Worse for this game specifically: the design puts the information war in
purchased mechanics — AEC buys sight, Meridian buys ambiguity, and the two oppose each other on one
legible axis — and geometric occlusion is a third information mechanic nobody authored, illegible at
RTS camera height, sitting underneath both. Radial vision is the classic-RTS answer because it is the
answer a player can hold: a circle, priced per template, visible in the fog's edge. The departure
condition is recorded in decision 4 and is deliberately concrete.

**Why memory lives in the kernel rather than the host.** Presentation-side memory — the interface
remembering ghosts, the way a human remembers — is cheaper and tempting. It fails both named
consumers: ADR 3004's AI reads a seat's view between ticks, and a host-side memory would make that
view a function of host history rather than kernel state, unreproducible in the re-run test; and a
saved match restored mid-run would wake with every player's memory blank, which for Meridian's
opponents deletes real information they had paid to acquire. State that decides — and remembered
positions decide where a player attacks — is hashed state.

### Rejected

- **A second, coarser vision grid** — a second resolution to keep registered; ADR 3001 decision 2's
  argument, inherited whole. Coarsening stays the fallback, by integer factor, if measurement asks.
- **Per-object pair visibility with no grid** — rejected above; shroud needs cells regardless.
- **Bounded-neighbourhood recompute instead of counters** — rejected above: correct only with a
  spatial source query the counter obviates.
- **Sampled-ray occlusion against the heightfield** — rejected above, with its reopen condition
  recorded in decision 4.
- **A fourth *state* for detection** — rejected by the design itself
  ([mechanics.md §5](../design/mechanics.md#5-vision-fog-and-detection)): concealment modulates
  *observed*, it does not extend the terrain-knowledge ladder, and a state enum would tangle the two.
- **Per-player storage with query-time team union** — rejected: pays the union on every read and
  lets allies' memories disagree.
- **Not hashing observed-now because its consumers hash** — rejected: true and insufficient;
  attribution is the point of per-subsystem hashes.

## Consequences

- `templates.json` gains `vision` now (this record is its consumer, on `unit` and `structure`,
  refused elsewhere — a prop that sees is a sensor wearing the wrong kind), and `concealed` and
  `detection` when their declaring content lands. [templates.md](../formats/templates.md)'s growth
  rule, applied three more times.
- A new subsystem enters the kernel's registration order behind the movers it watches, and the order
  becomes part of the contract, per the [determinism invariants](../invariants/determinism.md).
- Memory cost is real and scales with groups: two bit-planes plus two `u16` counter planes per group
  over the ground grid's cells. Typical maps are far below the 8192² format ceiling; the recorded
  mitigations are the integer coarsening above and, later, allocating detection planes only for
  groups that field a detector.
- The interface stops rendering the world and starts rendering a seat's view of it — shroud,
  fog-dimmed remembered ghosts, observed objects — which is presentation work M6 already expects.
- ADR 3004's omniscient-AI discontinuity closes when this lands: sweep results recorded before and
  after are not comparable, which that record already states.
- **The interface will knowingly show lies.** A remembered object may be gone; the ghost stays until
  someone looks. That is mechanics.md §5's feature, stated here as the unwelcome-looking consequence
  it is, so nobody "fixes" it.
- **Radial vision means a ridge hides nothing by itself.** Meridian's concealment and AEC's sweeps
  carry the entire hidden-information game until the reopen condition in decision 4 is met. If
  playtests find the terrain reading as tactically dead, that paragraph is where the argument
  restarts — with the cost figures this record declined to pay written beside it.
- Meridian's desk-serves-notice, "collapses once observed", AEC's sweeps, and false signatures all
  become expressible against one representation: discs, counters, memory, reveals. None is
  implemented here.

## What is left open

- **Whether a structure under construction sees.** A build site is a real object
  ([mechanics.md §4.2](../design/mechanics.md#42-construction)); whether its template's `vision` is
  live before completion is construction's call.
- **Air units.** ADR 3001 keeps air movers off the ground grid; whether a sortie contributes vision
  while it transits, and at what radius, belongs to the record that gives AEC its aircraft.
- **The exact per-seat query surface** — the read API the interface and AI share. Decision 8 fixes
  what it exposes (a group's bits, memory, and observed objects); its shape is implementation.
- **How a false signature writes memory** — a Meridian mechanic that plants a remembered object that
  was never there; the memory map supports the write, the mechanic and its price are faction work.
- **Reveal stamp garbage collection** — expired stamps are dead state; whether they are swept eagerly
  or lazily is implementation, noted only because dead state that reaches the hash would not be dead.
