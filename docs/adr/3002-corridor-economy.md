# ADR 3002: The corridor economy — one currency, three acquisitions, carriage on the map

**Status:** proposed

Numbered in the `3xxx` family — simulation — because an economy is kernel state and follows the same
determinism rules as everything else there. [ADR 3001](3001-pathfinding.md) set the precedent: a
gameplay mechanic whose implementation lives in `cic-sim` is a simulation record.

## Context

M6 charters the economy as one line: *"a resource, gatherers, and a rate that makes expansion a real
decision."* That is enough to build something and not enough to build the right thing, and the gap is
where this record goes.

Four constraints are already in force and none is negotiable here:

- **The setting is about passage, not territory.** Three belligerents contest a trade corridor, all
  three want the road open, and the war exists because "open" has three incompatible definitions. An
  economy of static mineral fields would contradict the premise the whole game is written on.
- **The faction bible has already specified three different economies.** AEC is "expensive, precise,
  air-dependent" with force arriving off-map; Concord is "slow to spin up, then unstoppable" with
  production as industry and a map presence that is literally paved; Meridian "holds route nodes and
  earns from traffic passing through, including the enemy's", with a hard rider — *"optional bonus
  income in contested places, never a gate on the other factions' economies — greed, not homework."*
  Whatever is decided here must implement those three, not replace them.
- **Balance needs commensurable prices.** Three factions cannot be compared if each holds a private
  currency, and [balance.md](../design/balance.md)'s budget line is arithmetic over one scale.
- **Simulation arithmetic is restricted.** [ADR 0007](0007-simulation-arithmetic.md): `f64`, a
  restricted operation set, and a kernel that is almost entirely integer.

There is also a named failure to avoid, and it is the reason this record was asked for. *Command &
Conquer 4* replaced its economy with an abstract point pool, removed base building and harvesting,
gated units behind persistent progression, and made losses free. The result is a game in which being
out of position costs nothing. Recording the alternative as a decision — with the rejected options and
their reasons — is what stops that outcome being reached later by a sequence of individually
reasonable simplifications.

## Decision

1. **The economy is carriage.** Goods enter the map at authored **gates** on the map edge, accumulate
   at **yards** along an authored route graph, and must be physically carried to a delivery point
   before they are worth anything. The atom is a **load**: a simulation object with a position and an
   integer value.

2. **Two layers, one currency.** *Freight* is on the map and contestable. *Credit* is a single integer
   per player, spendable, and **not** raidable once banked. A raid costs an opponent future income and
   a load in transit, never their bank.

3. **One currency with three names.** The engine calls it `credit`. AEC displays *Allocation*, Concord
   *Appropriation*, Meridian *Receipts*, through the existing string table. Identical value, three
   words, per the bible's lexicon rule.

4. **Three acquisitions, differing in ceiling, ramp and risk.** AEC flies loads to relocatable **pads**
   under a hard cap on simultaneous lifts raised by holding **beacons**. Concord hauls overland to
   large **plants**, with income rising as it grades roads. Meridian builds **permit desks** at route
   nodes and assesses a duty on every load that crosses, its own and its enemies' alike.

5. **Meridian's duty is additive.** The payer loses nothing: no slowed carriage, no locked yard, no
   drained bank. Meridian is paid *alongside*, not *out of*. Counterplay is fighting for the node or
   routing around it at a cost in time — a choice, never a chore. This is the bible's rider taken
   literally.

6. **A permit desk becomes visible to the payer once it assesses its first load.** Meridian's income
   reveals its position, which is the one thing the faction of concealment cannot conceal.

7. **A killed carrier drops its load where it died.** Recoverable by anyone. Interception pays the
   raider rather than merely costing the victim.

8. **Gates are neutral, indestructible, and authored.** Flow never stops *permanently*, so no match
   ends in mutual exhaustion; flow is fixed, so taking more means taking it from somebody. A link
   stops while it is being fought over, per decision 15.

9. **Carriers run standing round trips.** The player chooses the yard, the delivery point, and the
   escort. The player never routes an individual trip.

10. **Income is flow-limited rather than carrier-limited.** One carrier clears more than a yard
    supplies at reference distance, so the way to earn more is to hold more map. Beyond saturation a
    carrier is insurance against interception, and the saturation point is a *distance* rather than a
    count — a yard twice as far needs two.

11. **No second harvested resource.** A faction's second constraint is *structural* and specific to it:
    corridor capacity, queue capacity, standing convertible buildings.

12. **Every quantity is an integer**, with rates as an integer numerator over a fixed denominator and
    the remainder retained. Exact, driftless, hashable.

13. **The unit ceiling is a structure count**, so an opponent can lower it: pads and beacons, factories
    and queue slots, converted buildings still standing.

14. **A map's economic figures are published in its own metadata.** Gate count and rate, yard count,
    route node count, convertible buildings. Two of three factions have their ceiling set by map
    authoring, so a map is a balance surface and has to declare itself.

15. **A link where something is being harmed carries no freight.** The trigger is **damage dealt to a
    unit or structure that has an owner**, within the link's radius, held for a duration after the
    last such damage. Fire that harms nothing does not count, and neither does damage to neutral
    scenery, props, wrecks or unclaimed buildings — nor damage to the road itself, which is terrain
    and belongs to nobody. Flow accumulates upstream against a cap and the backlog moves as a surge
    when the link reopens.

    **Three separate things reduce a link's capacity, and they do not collapse into one.**
    *Interdiction*, above, is binary and clears when the fighting stops. *Condition* is continuous,
    moved only by the deliberate acts in decision 18, and clears when somebody repairs it.
    *Obstruction* is continuous, caused by accumulated wreckage, and clears as wrecks decay or are
    recovered. A battle on a road produces the first immediately and the third afterwards, and never
    the second.

16. **A supply chain is three layers and they are not interchangeable.** The **corridor** is authored
    road on the terrain; the **route graph** is authored topology over it; **carriage** is the
    player's own and is *not* bound to either. Carriers path over terrain like anything else and
    prefer the road because the road is cheap ground.

17. **Freight flows abstractly; the traffic on the road is presentation.** A load's journey from gate
    to yard is a rate along the graph, not a convoy of objects. The vehicles a player sees hold no
    simulation state.

18. **The corridor's condition is one continuous scale, and only deliberate acts move it.** Cell cost
    class from metalled down to rubble, with the link's capacity following it. Concord grades along a
    drawn path with an engineering unit; ordnance aimed at the road — artillery, rockets, an engineer's
    charge — craters it; **ordinary combat does not degrade it at all**. A cratered road still carries,
    slower; only demolishing a *structure* on the route severs it. Everyone can repair and Concord
    repairs best, because a map that degrades monotonically into unplayability is a map nobody
    finishes. There is no route-wide upgrade tier.

19. **AEC holds sortie slots, not carriers.** A slot is a recurring off-map sortie that flies to a
    yard, lifts a slung load, delivers to a pad and leaves. A downed sortie costs the load, the slot's
    downtime, and credit for the airframe. Entry is from the map edge nearest the pad it serves, so
    pad placement is the flight path.

20. **AEC's fallback is contracted ground haulage, paid by a fee per delivered load.** Outside corridor
    capacity entirely, so it runs alongside the air bridge rather than instead of it — and it
    **refuses contested ground**, which no order overrides.

## Rationale

**Why carriage rather than a rate.** A per-second trickle for holding a point is cheaper to implement
and produces no interceptable object, which removes raiding from the game — and raiding is the
mechanism that makes the map's middle worth anything. Carriage puts a killable thing on the road
between two places a player cares about, which is the same structure every RTS economy that worked has
used. Decision 7 is what makes it *pay*: a load that drops is loot, and a mechanic that only punishes
the victim is one players learn to skip.

**Why one currency and three acquisitions.** This is the load-bearing trade in the record. Asymmetric
economies are the bible's requirement and commensurable prices are balance's requirement, and they look
opposed until the split is put in the right place: **the asymmetry goes in the acquisition, the
symmetry in the unit of account.** AEC, Concord and Meridian earn in three genuinely different ways and
then shop in the same shop.

**Why Meridian's duty is additive and not extractive.** An extractive tax is the obvious design and it
is wrong twice. It makes the counterplay a chore — an opponent's response to being taxed is
maintenance, which is the failure the bible's rider names. And it inverts Meridian's character: a
faction that strangles traffic is not a faction that issues permits. Additive duty produces the
property that makes Meridian worth playing — **it profits from its enemies prospering**, so it holds a
standing decision not to kill a logistics line it could kill. That is the Chain's fault line, which the
bible flags as the faction's central contradiction, available in a skirmish instead of only in a
briefing.

**Why gates rather than depletable fields.** Depletion forces expansion, which is wanted, and produces
resource-death stalemates, which are not. Edge gates give a flow that is *positionally* finite and
never exhausted — and the fiction supplies it for nothing, because the corridor connects two places
that are not on the map. Yards accumulating what nobody collects is the second half: an unworked corner
of the map is stored value rather than wasted flow, so arriving late at a contested yard is still worth
doing.

**Why no second currency.** Every candidate — power, fuel, ammunition, manpower — makes the three
factions *more* alike, because all three then gather two things. A structural constraint makes them less
alike, and the bible has already specified three different ones. A second currency also doubles the
balance surface for a single extra axis of decision.

**Why a fight stops the link, and why the trigger is harm rather than fire.** The mechanic exists
because a supply line that keeps running through a battle is a supply line nobody has to think about.
Stopping it produces three things at once: the middle of the map becomes expensive to fight over,
because central links feed both players and a positional grind is mutual economic destruction; the
efficient economic attack becomes a **deep raid** at a link that feeds only the enemy; and a defender
cannot reopen the road by winning quickly, because damage to *either* side's assets holds it shut. The
road reopens when the fighting stops, not when it is won.

The trigger went through three versions. **Presence** was rejected first: one scout parked on a road
closes it forever, which is a chore and an exploit. **A weapon discharge** was rejected next, and it is
the version worth recording because it looked right — it lets a player close a road by shooting a
hillside, which is both exploitable and nonsense. A road does not close because somebody shot a rock.
**Damage to something owned** is what survives: a link closes only inside an engagement where an actual
asset is being destroyed, so the closure always costs what it should, and the exclusion of neutral
scenery, wrecks and unclaimed buildings is part of the rule rather than an implementation detail.

The corollary — a link with nothing on it cannot be closed — is accepted rather than worked around. It
is not the case that arises, because what a deep raid finds on a supply line is *carriers*, and a
carrier is an owned object whose destruction both takes the load and shuts the link. The raid's target
and the interdiction trigger are the same object.

**This does not breach decision 5.** That rider constrains Meridian's duty, which must never be
extractive. Interdiction is combat, available to all three equally, and paid for in a fight — which is
the only currency this design accepts for an economic attack.

**Why fighting, road damage and wreckage are three mechanisms rather than one.** The tempting
simplification is to let combat degrade the road and have one quantity carry everything. It is wrong
three times. A firefight over a stretch of tarmac leaves the tarmac, so it is *false*, and a design
whose model of the world is visibly false spends the player's trust for nothing. It makes every
skirmish a permanent economic act, so a map decays monotonically toward unplayable with nobody having
chosen it. And it collapses two attacks that should be distinct choices — attacking the traffic and
attacking the roadway — into whichever one is cheaper.

Separating them costs nothing and buys the shape: interdiction is immediate, binary and free of
ordnance; cratering is deliberate, gradual, persistent and paid for; wreckage is the delayed physical
consequence of the fighting, and it clears itself. **Fighting on a supply line therefore costs everyone
twice** — once while it happens, and again for as long as the debris sits there — which is the property
worth having, arrived at honestly.

Damaged and destroyed are also not the same state, and conflating them was the other half of the error.
A cratered road that stops carrying makes artillery a delete button aimed at the map; a cratered road
that carries *less* makes it an exchange, and leaves severing a link to the two things that should cost
real commitment — demolishing a structure, or going there and fighting. It also promotes bridges into
the map author's chokepoints, which is a lever worth having deliberately.

**Wrecks stamp a cost class rather than a footprint**, for the same reason. A footprint is a hard wall,
and one dead truck should not be one. What closes a road is accumulation, which is the thing that ought
to be true, and it makes Meridian's recovery crews into the map's road-clearing crews without anybody
adding a road-clearing mechanic.

**Why carriage is not bound to the route graph.** A trade route whose carriers run on rails — *Age of
Empires III*'s is the clearest reference — is interceptable only on the rail, which deletes the spatial
game, and it cannot express a faction that regrades the map, because the route is fixed by authoring.
Decision 16 keeps the authored road as *terrain* and lets pathfinding do the rest.

**Why grading is spatial and there is no upgrade tier.** A purchased "all roads are faster" is a menu
entry that appears nowhere on the map, which charter rule 6 forbids and which throws away the only
mechanic that makes "its map presence is literally paved" visible. Cratering exists as its counterpart
because Concord's economy *is* the map, so the attack on that economy has to be an attack on the map —
and the exchange is priced in time rather than credit, which is the currency Concord actually lacks.

**Why AEC holds slots rather than aircraft.** A fleet of owned lift helicopters shuttling yard-to-pad is
Concord's hauler with different pathing: same ownership model, same queue, same loss-and-rebuild loop,
different terrain rules. It would make AEC's economy a reskin of another faction's rather than an
expression of its own doctrine. The bible is unambiguous that force arrives from off-map through pads
and drop zones, and freight arriving the same way through the same mechanism is what makes the economy
and the reinforcement one system — which is in turn what makes air denial existential rather than
inconvenient.

**Why contractors, and why they are not simply better.** Total air denial otherwise kills AEC, and the
bible forbids a faction being characterised as helpless against anything. The alternative considered
seriously was **no fallback at all** — the answer to an air-defence net is to destroy it, which is what
an air force does, and a faction with a tool is not helpless. That option is cheaper, sharper, and was
the recommendation; it was declined in favour of the fallback being **the player's choice**, on the
grounds that trucks which are slower, more fragile and less profitable are a decision rather than a
crutch.

Three structural guards keep them a fallback rather than the main economy, none of them a tuning
number: the fee per load puts somebody else's margin permanently inside the income; the refusal to
enter contested ground makes them unavailable in exactly the situations that decide matches; and
because they consume no corridor capacity they compete with nothing, so a player who has the air bridge
has no reason to substitute. It also composes into a clean triangle — air defence beats the air bridge,
interdiction beats the contractors, the air bridge flies over interdiction — so no single investment
shuts AEC's economy down.

**Why the traffic is cosmetic.** Promoting it to simulation objects makes it killable, and destroying
commercial traffic to starve Meridian's duty is a legitimate strategy in this setting rather than a
gimmick. It is also a decision about civilian death, and this project's standard for those is that they
are made deliberately. Cosmetic traffic keeps the corridor visibly alive at no cost to the tick and
leaves the decision available.

**Why integers.** ADR 0007 makes this nearly automatic, but there is an economy-specific argument:
income accumulates over tens of thousands of ticks, and an accumulator is the worst possible place for
representation drift. An integer numerator over a fixed denominator is exact for the life of a match.

### Rejected

- **An abstract point pool, refreshing over time** — CnC 4's model. Rejected because nothing about it
  can be attacked, so it removes the map from the strategy. It is also the cheapest thing to build,
  which is why it needs a record rather than a preference.
- **Extractive taxation for Meridian** — rejected in the rationale above. It is the intuitive reading
  of "taxes throughput" and the bible pre-emptively forbids it.
- **Depletable resource fields** — rejected for stalemate, and for having nothing to do with a corridor.
- **A second harvested resource** — rejected for homogenising the factions.
- **Per-faction currencies** — rejected because [balance.md](../design/balance.md) becomes unwritable.
  This one is worth naming because it is a *tempting* way to express asymmetry and it forecloses the
  ability to check any of it.
- **Player-routed convoys** — rejected because per-trip micromanagement produces none of the decisions
  the model exists to create and spends attention the battle needs. The player decides where the
  endpoints are; the carrier decides how to get there.
- **Raidable banked credit** — rejected because retroactively deleting a saved plan is not something a
  player can play against. Raids take income and cargo, not savings.
- **Persistent between-match progression affecting capability** — rejected outright, and it is a
  charter rule rather than a balance position. Capability earned outside a match is capability the
  opponent cannot contest inside it. Cosmetics, records and campaign content carry the retention goal
  instead.
- **Presence as the interdiction trigger** — rejected: a single parked scout closes a road forever.
- **A weapon discharge as the interdiction trigger** — rejected, and worth recording because it was the
  first version written and it reads as correct. It lets a player close a road by shooting at scenery.
- **Rail-bound carriers on a fixed trade route** — rejected. Interceptable only on the rail, and
  incompatible with a faction that regrades the map.
- **A purchased route-wide grading upgrade** — rejected against charter rule 6, and because it deletes
  the one mechanic that shows a faction paving the map.
- **AEC owning lift aircraft** — rejected as Concord's hauler with different pathing.
- **No AEC fallback at all, SEAD as the whole answer** — a strong option, and the one recommended. It
  was declined deliberately in favour of contractors being a choice the player makes; recorded here
  because it is the version to return to if the fallback proves to be the main economy.
- **A deliberate low-cost "block the road" ability** — rejected. Interdiction should always cost a
  fight; a cheap roadblock turns a strategic decision into upkeep. The deliberate version is cratering,
  which costs ordnance.
- **Combat degrading the road** — rejected, and it was in the first draft of this record. It is false
  to the world, it decays every map monotonically toward unplayable, and it collapses two attacks that
  should be separate choices.
- **A cratered road being a closed road** — rejected. It makes artillery a delete button aimed at the
  map. Degradation is an exchange; severance costs a demolished structure or a fight.
- **Wrecks as impassable footprints** — rejected. One dead truck is not a wall; accumulation is what
  closes a road.

## Consequences

**It amends ADR 3001, three times.** [ADR 3001](3001-pathfinding.md) says a graded road is "a cell
class cheaper than ground" while setting plain ground at class `1` and step cost at `10 × class`, so
cheaper-than-ground is unrepresentable — grading could restore mud and never improve past it, which
flattens Concord's entire economic identity. A cell's class also ranks paths only; for Concord's income
to rise as it paves, the same class has to reach the movement rate. And decision 4 leaves open whether
an object stamps a footprint or a cost class, which the obstruction rule settles: a wreck stamps a
**cost class**, because one dead truck is not a wall. All three are recorded as amendments on that
record and carry this one's `proposed` status.

**The best property here was not designed.** Decision 15 meeting Meridian's already-written economy
produces a faction whose two income sources move in *opposite* directions along one axis: duty falls as
fighting rises, salvage rises with it. That is the Council and the Chain — recognition against a war
economy that cannot survive peace — as arithmetic a player feels in their income rather than as a line
in a briefing. It also carries a balance obligation, since two anti-correlated curves cannot be swept
one at a time.

**What this obliges.** Eighteen engine requirements, listed with their homes in
[mechanics.md §10](../design/mechanics.md#10-what-this-document-obliges-the-engine-to-gain). Six are
already promised or built — templates growing fields with the mechanics that read them, footprint and
passage from ADR 3001, runtime cell-cost edits, the string table, per-instance tint, and standing orders
that a carrier round trip extends rather than invents. Five are new: scenario `routes`/`gates`/`yards`,
integer credit accumulation, wreck objects with decay, convertible neutral structures, and detection as
an axis beside vision.

**A scenario format change.** `map.json` gains three optional members. Additive, so existing maps stay
valid, and the package loader is where cross-references are checked — the same reasoning that already
puts scenario-versus-terrain bounds checking there.

**Fog gains a fourth concept.** Detection is an axis beside shrouded/fogged/observed rather than a
fourth state, because a concealed object inside a player's vision is a different thing from an object
outside it. This is what makes AEC's purchased vision and Meridian's purchased ambiguity oppose each
other on one axis instead of being two unrelated gimmicks.

**Wrecks become simulation state.** ADR 0008 keeps the *tumble* in presentation, where it may differ
between clients. The wreck object that Meridian recovers may not, so it is in the kernel and the two
halves have to stay separated — the tumble is spectacle, the wreck is state.

**Map authoring becomes a balance activity.** Decision 14 makes it declared rather than accidental. A
node-dense map is a Meridian map and a town-free map is one Meridian cannot play, and neither should be
discoverable only by losing on it.

**Credit conservation becomes checkable, and should be checked.** Total credit created must equal gate
output collected plus duty assessed. Folded into the tick hash, an income bug that quietly doubles a
rate diverges on the tick it happens instead of surfacing as "matches feel fast lately".

**What is left open.** The measured numbers — duty rate, corridor capacity per beacon, batch size,
porter throughput, salvage cost and wreck decay — are swept against targets rather than chosen, and the
targets are in [balance.md §5.4](../design/balance.md#54-numbers-that-are-measured-not-chosen). The
grid as a map-level utility is designed and unscheduled. Three-way and team economics are untouched:
Meridian taxing two opponents at once is a qualitative change, not a scaling one.

**The unwelcome consequence.** This is more machinery than a trickle economy, and it lands inside the
milestone that also owes pathfinding, combat, construction, production, fog and an AI. The mitigation
is that it decomposes: gates, yards and carriage are a working economy on their own, and the three
faction-specific acquisitions are three separable increments on top of it. It is worth stating plainly
that decision 1 alone is the minimum viable version, and that the order to build it in is *shared
carriage first, faction divergence second*.
