# ADR 3003: Formation movement — the shape a group is already in, carried to where it is sent

- Status: accepted, implemented — direction set by Denys on 2026-08-02, built the same day as
  `cic_sim::formation` and the `move_group` verb.

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
- **Still not here**: a facing for the formation (decision 6), and *attack-move* and *patrol*, which
  are order kinds rather than movement and belong with combat. A group order also does not keep the
  group together *en route* — each member walks its own route at its own speed, so a mixed-speed
  group arrives strung out. Matching pace to the slowest member is a real want and a separate
  decision, because it is about what an order *means* rather than about where it ends.
