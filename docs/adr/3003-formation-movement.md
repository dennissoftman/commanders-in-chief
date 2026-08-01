# ADR 3003: Formation movement — the shape a group is already in, carried to where it is sent

- Status: accepted, implemented — direction set by Denys on 2026-08-02, built the same day as
  `cic_sim::formation` and the `move_group` verb. **Two amendments, both accepted and both built**,
  from the same direction the day it landed: **A** gives a group order a facing the player drags,
  which supersedes decision 6; **B** holds a column to the pace of its slowest member, which closes
  the consequence this record left open.

## Context

[M6](../milestones/m6-gameplay.md) charters *selection and orders: move, attack, attack-move, stop,
hold, patrol, and **formation movement***. Everything before it in that line is built or scoped;
this is the last of the movement half.

It is also a debt [ADR 3001](3001-pathfinding.md) named against itself. Decision 10's local
avoidance keeps units out of each other, and its own record admits the limitation: sixteen units
ordered to one point stop stacking but keep *jostling*, because every one of them is still walking
at a point every other one is standing on. Measured on the rough test map, sixteen units sent to one
cell by sixteen orders end with **twelve of a hundred and twenty pairs** overlapping and never
settle. The record names the fix and does not build it: *"sixteen units sent to one place should be
given sixteen places."*

**Direction set by Denys, 2026-08-02**, and it rules out the obvious implementation as much as it
asks for one: **free** formation, not a box and not any other fixed shape; **not random**; wide units
**placed efficiently**; and the result visible on the map. The comparison offered was *Command &
Conquer 3*, whose right-click keeps a group's own arrangement rather than pouring it into a template.

Constraints not this record's to revisit: [ADR 0007](0007-simulation-arithmetic.md)'s operation set,
the [determinism invariants](../invariants/determinism.md), and ADR 3001's decision 9 — units store
no heading, so no part of this may need one.

## Decision

1. **A group move is its own verb.** `move_group` names a set of units and one destination;
   `move` is untouched and still means what it did. Formation is a property of the *order*, not of
   the units — a group that knows it is a group is what makes a shape available to carry, and
   inferring one from "several move orders that arrived on the same tick with the same destination"
   would be a guess that is wrong the first time two players click the same rock.

2. **The formation is the one the group is already in.** Each member's slot is its own offset from
   the group's centre, translated to the destination. A line arrives as a line, a wedge as a wedge,
   a huddle as a huddle. Nothing is imposed, nothing is generated, and there is no shape table
   anywhere in the implementation. That is the whole of *free*.

3. **The assignment is the identity: member `i` gets slot `i`.** Nothing is matched, scored or
   shuffled. That is the whole of *not random*, and it is also the best possible answer to path
   crossing — on open ground every member's displacement is the *same vector*, so the group
   translates rigidly and no two units cross at all.

4. **Slots are opened out until none overlap.** The same radius-aware half-overlap push local
   avoidance applies to live units, iterated on the slots before anybody sets off. A group that was
   already spread out is untouched, because there is nothing to open; a group that set out standing
   in a heap arrives as a rosette. The correction each slot takes is the **average** of what its
   neighbours ask for rather than the sum, because summing overshoots and the group oscillates
   outward instead of settling. Opening a group out can drift its centre, so the whole set is slid
   back onto the destination afterwards — and only when something moved, so a group that needed no
   opening comes through the translation exactly.

5. **A slot the ground refuses is re-placed, widest member first.** The nearest cell centre that a
   unit could stand on and that clears the slots already placed, found by a ring search outward from
   where the slot wanted to be. **Descending radius is the point** and is the whole of "wide units
   placed efficiently": the member that has the hardest time fitting anywhere gets first refusal on
   the roomy ground, and the narrow ones fill in around it. Ordering by identifier instead would
   hand the one clear pocket to whichever unit happened to be built first and leave the wide one
   squeezed against a cliff.

6. **The formation does not rotate.** Translation only. A group ordered backwards keeps its shape
   rather than flipping, which is what a plain right-click means in the games this is modelled on; a
   facing would need an input the command does not carry and a heading units do not store. A
   *drag-to-face* order is a later verb if one is ever wanted, and it would be an addition rather
   than a change.

   **Superseded by amendment A**, which is the addition this clause predicted — and it was an
   addition rather than a change: a *click* still does exactly what this says.

7. **A selection is a set, not a list.** The named identifiers are sorted and deduplicated before
   anything is computed, so the answer depends on *who* was selected and not on the order a client's
   selection happened to iterate in. Two clients that agree about the selection then agree about
   where everybody goes, which removes a way for a lobby to desync on nothing worse than a hash set.

8. **Presentation draws the slots, and needs no new state to do it.** A unit's slot is the last
   waypoint of its route, so a marker per destination *is* the formation, drawn while it matters and
   gone once everybody has arrived.

## Rationale

**Carrying the shape, against generating one.** A generated formation — a box, a wedge, a
line-abreast — is the common implementation and it is what the direction excludes. It is worth
saying why beyond "not asked for": a template throws away information the player created. Somebody
who spread their army thin to cross a ridge, or bunched it to squeeze through a gap, has expressed
an intention, and a box formation deletes it on the next click. Carrying the offsets is also
*less* code than any template scheme, needs no per-faction or per-army-size table, and cannot
produce a shape the map has no room for.

**The identity assignment, against matching units to slots.** With generated slots you need an
assignment step, and the good ones — minimum total distance, minimum crossings — are the
assignment problem, which is `O(n³)` and needs a tie-break rule at every equal cost to stay
deterministic. Carrying offsets makes the question vanish: the slot a member belongs in is the one
derived from where it already stands. This is the same shape of argument as ADR 3001's integer
costs — choose the representation whose hard question does not exist.

**Opening out, against leaving the shape alone.** A group that set out in a heap has no shape to
carry, and translating a heap arrives as a heap; local avoidance would then untangle it, which is
exactly the jostling this record exists to stop. Opening the *slots* rather than the units means the
untangling happens on paper, before anybody walks, and costs nothing at all on the common case where
the group is already spread.

**Widest first, against nearest first or identifier order.** Only one of the three is a rule about
the thing that matters. Nearest-first is a greedy scheme that lets a rifleman take the clearing
because it happened to be standing closer; identifier order is arbitrary. Size is the property that
decides who *cannot* fit elsewhere, so it is the property to sort on, and it costs one comparison.

**Bounds rather than coefficients.** The spread loop's round limit, its overshoot, and the settle
search's reach are constants, not settings on a rules struct. Every other coefficient in this
subsystem is a setting and hashed, and the argument for that — a number that changes where units end
up changes the game — genuinely does not apply here: these three bound a *search for an answer* that
is the same answer either way, and a number nobody would ever tune is a number nobody should have to
carry through a hash. The one that is a near miss is the overshoot, which does move the final
spacing slightly; it is a constant because the alternative is a knob whose only honest setting is
"enough".

## Consequences

- `cic_sim::formation` is a **pure function** — members and a destination in, slots out — which is
  why its hard cases are cheap to test: a huddle, a slot in a lake, two members contending for one
  pocket. Nothing about it needs a kernel, a tick or a command.
- The command set gains one verb and no new state. `Units` holds nothing extra, hashes nothing
  extra, and a group order produces exactly what `n` move orders produce: `n` routes.
- **Measured**: sixteen units sent to one cell as a group end with **zero** overlapping pairs,
  against **twelve** for the same crowd sent by sixteen separate orders. That closes ADR 3001
  decision 10's recorded limitation.
- A group order costs one `O(n²)` relaxation and up to one ring search per refused slot, once, when
  the order lands — not per tick. At the group sizes an RTS selects this is not measurable.
- The demo viewer issues group orders and draws a plate at each slot, so the formation is legible
  before it arrives. That is presentation reading simulation state and adding nothing to it.
- **Still not here**: *attack-move* and *patrol*, which are order kinds rather than movement and
  belong with combat, and **selection**, which is input rather than simulation. The two things this
  list named as missing when it was written — a facing, and keeping the group together — are the two
  amendments below.

## Amendments A and B — accepted, and both built

Both came out of the same conversation as the record itself, on 2026-08-02, and neither reverses a
decision: A is the addition decision 6 predicted, and B is the consequence this record listed as
outstanding.

### A. A group order carries a facing, and the player drags it

**What the record got right and what it left short.** Decision 6 declined rotation because "a facing
would need an input the command does not carry" — which was true of the verb as written, and is a
statement about the verb rather than about the mechanic. Denys asked for the thing *Command &
Conquer 3* and *Red Alert 3* have, where holding the mouse and dragging chooses how the squad forms
up. That is the input decision 6 was missing.

**The order gains a facing**, as a **direction vector** rather than an angle. The shape is then
arranged for that heading instead of for the one the group happens to be marching on: it is rotated
by the turn that carries the direction of travel onto the dragged direction. A line abreast of an
eastward march, dragged north, arrives abreast of north.

- **A click is untouched.** No drag is no facing, and no facing is the pure translation decision 2
  describes. Nothing in this amendment can rearrange a formation the player did not ask it to, which
  is the whole reason it is an addition rather than a change.
- **A vector, not an angle, and this is the interesting part.** Turning the drag into an angle would
  need `atan2` on the host and a sine and cosine in the kernel — all three refused by
  [ADR 0007](0007-simulation-arithmetic.md) decision 3 — to arrive at exactly the number that was
  already in hand. The rotation from one unit vector to another *is* their complex quotient:
  `to × conj(from)` is the `(cos, sin)` of the difference, and applying it is four multiplies. Two
  square roots, two divisions, and **no trigonometry enters the crate**. The promise in
  `cic_sim::units` that it "needs no trigonometry until something wants a facing angle" survives
  something wanting a facing.
- **A drag of no length is a click**, and so is one that is not a number. Both fall out of the
  normalisation rather than being special-cased — the first version guarded them twice, and the
  second guard was measured changing nothing, which is the definition of a rule in two places.

**What this deliberately is not.** *Red Alert 3*'s drag also sets the *width* of a line the squad
forms into, which is a **generated** formation and is what decision 2 declines. The fork is worth
naming because it is a cheap one to take later: the same verb, the same payload, and a different
reading of the drag — its length as a spread rather than as nothing. It is not taken now because
generating a shape throws away the one the player made, and this record's whole position is that the
player's own arrangement is the thing worth keeping.

### B. A group marches at the pace of its slowest member

**The consequence this record listed and did not fix.** Each member walked its own route at its own
speed, so a group of mixed speeds arrived strung out — a formation that existed at the moment of the
order and nowhere after it.

**A unit now carries two speeds**: `speed`, what the template says, and `march`, what it is walking
at under its current order. A group order sets every member's `march` to the slowest `speed` in the
group; a single order and a stop set it back. Hashed, like everything else that decides where a unit
ends up.

- **The ground class still multiplies it**, so [ADR 3001](3001-pathfinding.md) amendment B survives:
  a metalled road speeds the whole column rather than stretching it out again. It is a *base* speed
  that is held, not the final rate.
- **The slowest of the members actually going**, so somebody else's unit in a selection cannot slow
  you down — it was rejected before the pace was taken.
- **It does not make the group arrive together**, only stop members running away from each other:
  two units with different route lengths still arrive at different times. Matching *arrival* would
  mean scaling each member's speed by its own route length, which makes a unit crawl because
  somebody else went the long way round. Not done, and named here so it is a decision rather than an
  oversight.

## How this method is checked, and how another one would be

Worth its own section, because "is this the right method" is not a question tests answer by passing.
Three different things settle three different parts of it, and only the first is automatic.

1. **Properties**, by assertion: the shape survives, nobody crosses anybody, no slot lands on ground
   nothing can stand on, the same group always lands the same way. Those are settled, and they are
   what the unit tests beside `cic_sim::formation` are for.
2. **Behaviour over a whole map**, by measurement. `cic-sim/tests/formation.rs` sends an eight-unit
   squad to every eleventh passable cell of the rough test map and counts two things — how many
   slots come through as a **pure translation**, and how many the ground **refuses** before any
   repair. Measured at **2612 of 2728 slots untouched (95.7%)**, with the moved slots being
   *exactly* the ones the terrain refused and no others, and **116 refusals against a compact box's
   72**. So carrying the shape costs about **1.6 times** the repair of the tightest packing there
   is, and buys a formation that survives untouched 96% of the time. **That is the trade, in
   numbers.** A different method — a generated box, a wedge, a matched assignment — is scored by
   running it over the same suite, which is why the box is already in there as a yardstick.
3. **Whether it looks like an army moving**, by watching it. Nothing here settles that, and this
   project has been caught by the gap before: the "clunky movement" complaint that produced
   [ADR 3001](3001-pathfinding.md)'s amendment D passed every assertion it had. The viewer patrols
   two groups and draws a plate at each slot so that there is something to watch.
