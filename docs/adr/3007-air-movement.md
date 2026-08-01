# ADR 3007: Air movement — off the grid, in a straight line, at no altitude

- Status: proposed

## Context

An accepted record has already built an economy on movers nobody has defined. [ADR
3002](3002-corridor-economy.md) decision 19 gives AEC sortie slots whose sorties fly map edge → yard →
pad and leave; [mechanics.md §2.5](../design/mechanics.md#25-the-three-acquisitions) tables that route
as "Straight, ignores terrain"; interception of airframes is the counterplay the whole acquisition is
priced around, with a downed sortie costing three ways. [balance.md
§3.3](../design/balance.md#33-modifiers) prices "cannot engage ground, or cannot engage air" at ×0.6
and [§3.1](../design/balance.md#31-durability) carries an `air` durability row of 1.23;
[mechanics.md §3.2](../design/mechanics.md#32-damage-types-and-armour-classes) has an `air` armour
class taking frag at 125; [faction-mechanics.md
§3](../design/faction-mechanics.md#3-rules-that-apply-to-all-three) requires anti-air in every
faction's roster. Issue #76 item 5 names the gap: nothing records how any of this moves, occupies
space, is targeted, sees, or enters the map.

The boundaries around the gap are already drawn, deliberately, from three sides. [ADR
3001](3001-pathfinding.md)'s Consequences: *"Air movers are outside this record entirely. AEC's
aircraft, when they exist, do not consult the ground grid; their record owns what they consult
instead."* [ADR 3005](3005-route-graph.md): *"ADR 3002 decision 19's sorties traverse no links, and
this record inherits that as a boundary: nothing here applies to AEC's carriage leg."* And [ADR
3006](3006-vision-and-fog.md) left one item open by name: whether a sortie contributes vision while it
transits, and at what radius, "belongs to the record that gives AEC its aircraft". This is that
record. It gives the excluded mover its rules and closes the open item; it does not touch the slot
economy, which stays ADR 3002's.

Constraints in force and not revisited: [ADR 0007](0007-simulation-arithmetic.md)'s operation set;
the [determinism invariants](../invariants/determinism.md); [ADR 0008](0008-physics-engine.md), from
which nothing arrives — physics is cosmetic, so no flight model exists to consult;
[templates.md](../formats/templates.md)'s rules that a `unit` requires `speed` and `radius`, refuses
`footprint` and `passage`, and grows a field only with its first consumer.

## Decision

1. **An air mover is a unit off the grid.** It is `kind: unit` with one new template field,
   `mover: "air"`; absent means `"ground"`, any other value is refused. An enumeration rather than a
   boolean `airborne`, because [ADR 3001](3001-pathfinding.md) decision 3 already left per-class
   interpretation of movement open — "one mover class for now" — and a boolean spends the whole
   axis on the first departure from it. The format's own style settles the rest: unknown values are
   loud errors, absence is the common case, and a field that names what it means reads better in a
   hand-edited file than a flag beside a kind.

2. **An air mover has a 2D position like everything else, and no altitude in simulation state.**
   This is ADR 3001 decision 9's facing argument, reapplied: state nothing reads is state that can
   desync for nothing. Nothing in the design reads a height — the `air` armour class already carries
   everything "being airborne" means to combat, and an altitude number would be a third information
   channel beside class and position that nobody prices and no mechanic consults. Altitude is
   presentation, exactly as deck height on a bridge is: the viewer draws a sortie at whatever height
   reads well, freely, per client. **The reopen condition, recorded:** the day a mechanic reads
   height — terrain-hugging under a radar floor, a high-altitude band AA cannot reach — altitude
   enters unit state with that mechanic, as its record's integer, and this paragraph is where the
   argument restarts.

3. **Movement is the straight line the ground stepper already had.** Before routes existed,
   `cic_sim::units` moved a unit in a straight line — a subtraction, a `sqrt`, a division, all in
   ADR 0007's permitted set — and that stepper was kept as the thing that walks waypoints. An air
   mover is that stepper with one waypoint: no route, no repath, no cost classes, no corner
   rounding. ADR 3002 decision 19's "straight, ignores terrain" becomes literal. An air mover never
   consults `Ground`: passability does not apply, the pace multiplier of ADR 3001 amendment B does
   not apply (there is no cell class under a thing that is not on the ground — its `speed` is its
   speed), and a grid edit never repaths it, because it holds nothing a stamp could invalidate. That
   is 3001's exclusion respected as a seam rather than skirted: the grid's consumers are ground
   movers, whole and only.

4. **Air occupies no ground.** No `footprint`, no `passage` — [templates.md](../formats/templates.md)
   already refuses both on every `unit` and air changes nothing there — and no participation in
   ground local avoidance: an air mover neither pushes a ground unit nor is pushed by one. A tank
   and a helicopter at one position is fine; they are at different heights the simulation does not
   model, which is decision 2 paying for itself immediately.

5. **There is no air-air avoidance, for now.** Sorties fly authored point-to-point legs and rarely
   meet; two that overlap mid-flight are resolved visually, by presentation offsetting what it
   draws, which is presentation's licence. The quadratic pair loop stays ground-only, so adding air
   movers does not grow it. **The reopen condition, recorded:** the day orderable air fleets stack
   on one target and jostle there, ADR 3001 decision 10's circle push extends to an air plane of its
   own — the same measured pass, run over a second roster, with the ground never in the pair set.

6. **`radius` stays required on an air unit.** The requirement is one rule for every `unit` and it
   keeps meaning something here: the radius prices the target circle — how big a thing an anti-air
   round has to find, what a click has to hit for selection — even while no push consumes it. The
   alternative, waiving it for `mover: "air"` until decision 5 reopens, saves one number per
   template and costs a conditional in the one validation rule templates.md states twice as "the
   identical rule as `speed`". A field that is briefly consumed by less than it will be is cheaper
   than a rule with an exception in it.

7. **A sortie enters at the map edge and leaves off it, as ordinary object creation and removal.**
   Spawn and despawn go through the id allocator like every spawn today — deterministic,
   identifier-ordered, hashed on the tick they happen. The entry point derives from the pad the
   sortie serves, per ADR 3002 decision 19's "enters from the map edge nearest the pad": take the
   pad's cell by the same index arithmetic everything on the grid uses, compare its whole-cell
   distance to each of the four edges as integers, and the smallest wins — **ties break in declared
   order: west, east, south, north**, an arbitrary rule stated so it cannot be an accidental one.
   The spawn point is the pad's coordinate projected perpendicularly onto the winning edge; the exit
   is the same derivation from wherever the leg ends. **An air unit mid-flight is ordinary kernel
   state, and there is no off-map limbo object.** A slot whose sortie is absent is represented by
   the slot's recovery timer — ADR 3002's downtime, a counter, not a position. Off the map, an
   airframe is a number; on it, an object; the boundary between the two is a spawn.

8. **An air mover is targeted as an ordinary positioned object.** A weapon that can engage air —
   balance §3.3's domain split, priced at ×0.6 for a weapon that can do nothing else — range-checks
   it by the same squared-distance comparison as everything else, from world positions, with no
   altitude term (decision 2 means there is none to add). Combat's record owns the shot, the roll
   and the reveal; this one owns only that the target is there to be shot. A downed carrier-sortie
   drops its load where it fell — ADR 3002 decision 7, inherited without amendment.

9. **An air mover with `vision` contributes exactly like a ground source.** This closes ADR 3006's
   open item: that record's disc machinery is source-agnostic — a disc of cells, a per-cell counter,
   increments and decrements on cell crossings — and an air mover's position crosses cells like
   anyone's. No new mechanism, no air-specific radius rule; whether a sortie's template declares
   `vision` at all, and at what figure, is faction and balance work. Air is observed under the same
   three states as everything else, and **`concealed` may appear on an air template**: nothing
   forbids it, the detection axis is orthogonal to the mover axis, and a stealth flight is a
   legitimate future faction mechanic that costs this record nothing to leave representable.

10. **Hashing and determinism: nothing new.** An air mover is ordinary hashed unit state — position,
    speed, radius, route of one leg — under the subsystem hash that already exists. What matters is
    what this record does *not* add: no second movement arithmetic (the stepper is the same code on
    the same operation set), no new random stream (a straight line draws no randomness), no new
    container, no new registration. A record that adds a mover class and no determinism surface is
    the cheapest kind this family gets.

## Rationale

**Why no altitude, against altitude as simulation state.** The tempting version is a `z` beside `x`
and `y` — it is "obviously" what airborne means, and it would be hashed, replayed and networked for
nobody. No accepted or proposed mechanic reads it: combat's domain split is the armour class, vision
is radial by ADR 3006 decision 4, terrain does not block flight by ADR 3002's own table. A number in
state that nothing consumes is pure desync surface, which is the exact argument that kept facing out
of unit state in ADR 3001 decision 9 — and that decision has since been vindicated twice, by
presentation deriving headings happily and by formation facing arriving as a direction rather than an
angle. When a height mechanic arrives, its record adds the field with its consumer, which is this
project's growth rule applied to state instead of formats.

**Why not the ground grid with an all-cells-passable overlay.** The seductive unification: air is
just a mover class whose cost table says every cell is class 1, and one movement system serves
everyone. It is wrong structurally, not just wastefully. A route over a uniform grid is a straight
line that paid A\* to find it — search, string-pulling, corner rounding and repath-on-edit all run to
produce what a subtraction already knew. Worse, it couples what 3001 deliberately severed: every grid
edit would test air routes for intersection (finding none, every time, for ever), the air mover's
motion would sit downstream of `GroundRules` and the grid fingerprint, and a pathfinding change could
move an economy built on sorties — the same cross-subsystem attribution smear ADR 3005 refused when
it declined to derive link geometry from the grid. The grid's whole contract is "consult me and I
will repath you"; a mover that must never be repathed does not belong on it.

**Why no air-air avoidance now.** The push pass exists to stop orderable crowds stacking, and the
first air movers are not orderable and do not crowd — a sortie flies its authored leg and leaves. The
overlap case is visual, so presentation resolves it, which ADR 0008 already licenses. Building the
air plane now would double the pair-loop surface for a roster of zero.

**Why no limbo object for an absent sortie.** An off-map sortie could be an object at a sentinel
position, ticking home. Every consumer would then need to know the sentinel: vision must not count
it, targeting must not acquire it, the hash carries it, and a bug that lets one interact is a ghost
in the fog. ADR 3002 already gives the absence a representation — the slot's downtime — and a timer
cannot be shot, seen, or desynced into play. Objects exist on the map or not at all.

### Rejected

- **Altitude as simulation state** — rejected above; the reopen condition is decision 2's.
- **Air on the ground grid via an all-passable overlay** — rejected above; the load-bearing
  rejection in this record.
- **Air-air avoidance in the first slice** — rejected above; reopen condition in decision 5.
- **An off-map limbo object** — rejected above; the slot timer is the representation.
- **A boolean `airborne` instead of `mover`** — rejected in decision 1: it spends the mover axis on
  its first value.
- **Waiving `radius` for air units** — rejected in decision 6: an exception to a rule stated once is
  dearer than a field consumed by less than it will be.
- **Deriving the entry edge from the flight's target rather than the pad** — rejected: ADR 3002
  decision 19 already says the pad is the flight path, and "pad placement draws the interception
  surface" is faction design this record must not quietly re-decide.

## Consequences

- `templates.json` gains `mover` — with the first air template, per the growth rule; this record is
  the decision and AEC's sortie work is the consumer. `footprint` and `passage` stay refused on
  every unit; nothing in this record touches that rule.
- The stepper grows a branch, not a sibling: `mover: "air"` skips route planning, repath, ground
  pace and the ground pair loop, and shares everything else. One movement arithmetic, two consumers.
- **Air ignoring terrain and interdiction makes deep raids trivially safe from ground denial.** A
  sortie crosses any front, any crater, any severed bridge, and no ground stance answers it — the
  counterplay is anti-air, whole and only, and a map position with no AA coverage has no answer at
  all. That is the design's own intent — faction-mechanics §3 puts anti-air in every roster and the
  air bridge is priced around interception — but it is worth stating as the sharp edge it is: every
  balance sweep of AEC's economy (balance §5.4's sortie rows) is implicitly a sweep of how much AA
  the reference map's matches actually field.
- ADR 3006's open item closes on acceptance: an air source is a source. The remembered-object and
  reveal machinery apply unchanged, so a fogged sortie is remembered wrong like anything else.
- [ADR 3003](3003-formation-movement.md) does not extend to air: its slot repair consults the grid
  ("a slot the ground refuses is re-placed"), which is meaningless for a mover the ground cannot
  refuse. A `move_group` naming air units is out of scope until orderable air exists; its record
  decides whether slots even apply or a flight simply arrives in the shape it flew.
- Spawn-at-edge becomes the first spawn whose position is derived rather than commanded, and the
  derivation (decision 7) is deterministic by integer comparison with a declared tie order — one
  more rule in the family of "ties break on the lower cell index".

## What is left open

- **The sortie slot economy** — ADR 3002 decisions 19–20 own slots, timers, costs and the
  contractor fallback; this record gives them a mover and nothing else.
- **Orderable combat aircraft** — representable now; whether AEC fields a gunship is faction work,
  and roster content brings the `vision`, `concealed` and weapon figures with it.
- **Anti-air weapon statistics** — combat's record and balance's tables; §3.4's worked anti-air row
  already prices one.
- **Formation movement for air** — noted above; waits on orderable air.
- **Weather affecting flight** — mechanics §8's parked item; an air economy degrading in low cloud
  would be that item landing, not this record growing.
- **The falling airframe** — the kernel decides the death and drops the load; the tumble is ADR
  0008's presentation, free to exaggerate.
- **Altitude and air-air avoidance** — each has its reopen condition recorded in its decision,
  which is where those arguments restart.
