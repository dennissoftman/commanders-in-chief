# Commanders in Chief — Game Mechanics

**Status:** working draft. The economy it describes is **accepted** as
[ADR 3002](../adr/3002-corridor-economy.md), so what is drafty here is the writing and the measured
numbers rather than the shape.
**Scope:** the rules every faction plays under. What a resource is, how force is produced, how combat
resolves, what a player can see, and how a match ends.

**Companion documents.** [faction-bible.md](faction-bible.md) is the character specification and is
*upstream* of this file — where it states a doctrine, this document is obliged to implement it, not
free to reinterpret it. [faction-mechanics.md](faction-mechanics.md) is how each faction expresses
the rules below. [balance.md](balance.md) is how the numbers get set and checked.

---

## 0. The one paragraph that matters

Three belligerents contest a trade corridor, and none of them is fighting for territory — they are
fighting over the terms of passage. So **the economy is passage.** Goods enter the map at its edges,
accumulate at yards along the road, and have to be physically carried somewhere before they are worth
anything. All three factions get paid out of that flow and each one takes its cut in a different
place: AEC flies loads out, Concord hauls them overland on roads it built, and Meridian charges a duty
on other people's carriage. One currency, three ways to earn it, every one of them a thing standing on
the map that an enemy can shoot.

Everything in this document is a consequence of that sentence or a constraint on it.

---

## 1. The charter: what this game refuses to be

The failure this section exists to prevent is specific, and naming it is cheaper than rediscovering
it. *Command & Conquer 4* removed base building, removed harvesting, replaced an economy with an
abstract point pool, gated units behind persistent between-match progression, and made unit losses
free. Each of those is individually defensible and together they removed the reason to look at the
map. A player who cannot lose anything by being out of position is not playing a strategy game.

These are non-negotiable, and each one is testable rather than aspirational:

1. **There is a base and the player builds it.** Structure placement is an economic decision, because
   income is a function of carriage distance. A base in the wrong place earns less.
2. **Resources are on the map, not in a menu.** Income follows from position and from what a player
   holds, never from elapsed time alone.
3. **Carriers are units, and they can be killed.** Interception is a strategy. A load lost in transit
   **drops where the carrier died** and can be recovered by anyone, so raiding pays the raider rather
   than merely costing the victim.
4. **No between-match progression affects match power.** No unlocks, no account levels, no earned
   access. A first-time player and a veteran field the same options at minute zero. Cosmetics and
   records are fine; capability is not.
5. **No class-locked partial armies.** Every faction covers every tactical role — that is a bible
   constraint, not a balance preference — and no mode hands a player a third of the game and calls it
   a specialisation.
6. **Technology is bought inside the match and stands on the map.** Every tier a player advances is a
   structure an enemy can destroy, so an advantage is always takeable.
7. **The unit ceiling is a structure count, never an abstract pool.** Whatever bounds a player's army
   is something they built, can see, and can lose. See [§4.4](#44-the-ceiling-is-built-not-granted).
8. **Losses cost.** No free reinforcement, no unit that returns at no price. Attrition has to be felt,
   or none of the seven rules above matter.
9. **One currency.** Costs are quoted in the same units for everyone, so a designer can compare an AEC
   unit to a Concord one. Asymmetry lives in *how a faction earns and produces*, never in a private
   currency that makes its prices incommensurable.

Rule 4 is the load-bearing one and the easiest to lose to a good argument. Retention, progression and
a sense of investment are real goals with real advocates. They are met with cosmetics, records,
campaign content and ranked history — never with capability, because capability earned outside the
match is capability the opponent cannot contest inside it.

---

## 2. The economy

### 2.1 Freight and credit

| Layer | What it is | Who sees it | Where it lives |
|---|---|---|---|
| **Freight** | Physical goods, as discrete **loads** with a position and an integer value | Everyone, subject to fog | Simulation objects |
| **Credit** | The single spendable number | Its owner | An integer per player |

Freight is contested; credit is not. A load is taken by holding ground and carrying it; credit, once
banked, cannot be stolen, raided or drained. That split is deliberate: a raid should cost an opponent
*future* income and a load in transit, never their bank, because retroactively deleting a player's
saved-up plan is not a decision they can play against.

**Credit has three names and one value.** AEC calls it **Allocation**, Concord **Appropriation**,
Meridian **Receipts**. The engine calls it `credit`, the string table holds the three display keys,
and no faction ever uses another's word for it — the same lexicon rule the bible applies to
everything else. The underlying quantity is identical, because [balance.md](balance.md) cannot work
otherwise.

### 2.2 The three layers of a supply chain

"Supply chain" does three jobs at once here, and confusing them is the fastest way to design the wrong
thing:

| Layer | What it is | Owned by | What a player does with it |
|---|---|---|---|
| **The corridor** | A road on the terrain, drawn by the map author: cheap cell classes, visibly different ground | Nobody | Drives on it, improves it, damages it |
| **The route graph** | The authored topology over it — gates, yards, nodes, and the **links** between them | Nobody | Freight flows along it, duty is assessed on it, links can be interdicted |
| **Carriage** | Carriers moving loads from a yard to a delivery point | The player | The part that is built, escorted, and lost |

**The corridor is a fixed authored road; carriage is not bound to it.** That split is the decision, and
it is worth being explicit about the alternative: a trade route whose carriers run on rails — *Age of
Empires III*'s is the clearest reference — is interceptable only *on* the rail, which deletes the
spatial game, and it cannot express a faction that regrades the map, because the route is fixed by
authoring. Here a hauler paths over terrain like anything else and merely *prefers* the road, because
the road is cheap ground.

**Freight flows abstractly, and the traffic on the road is presentation.** A load's journey from gate to
yard is a rate along the graph rather than a convoy of objects. What the player sees moving on the
corridor is drawn by the renderer, holds no simulation state, and is therefore free — the road looks
alive at no cost to the tick and none to determinism.

That is a deliberate deferral and not a simplification. Promoting the traffic to simulation objects
makes it killable, and destroying commercial traffic to starve Meridian's duty is a real strategy with
real weight in this setting. It should be reached by a decision, not by somebody adding a health field
to a truck.

#### The corridor's condition is a scale, and only deliberate acts move it

Road quality is one quantity — the cell cost class, five discrete rungs: metalled, graded, plain,
mud, rubble. Grading moves a stretch toward metalled; ordnance moves it toward rubble. The
link's carrying capacity follows the scale, so this single number is where Concord's rising income and
everyone else's answer to it both live.

**Only deliberate acts move it.** Ordinary combat does not damage the road, and this is a rule rather
than an omission: a firefight over a stretch of tarmac leaves the tarmac. What degrades a road is
ordnance aimed at the road — artillery, rockets, an engineer's demolition charge — each of which is
paid for and aimed on purpose.

The tiers matter, because damaged and destroyed are not the same thing:

| Act | Effect on the road | Effect on the link |
|---|---|---|
| **Grading** | Toward metalled, permanently and publicly | Capacity rises |
| **Cratering** — artillery, rockets, charges | Toward rubble. Still passable, slower | Capacity falls, proportionally |
| **Demolition of a structure** — a bridge, a culvert, a cut | Impassable at that point | Capacity zero until rebuilt |
| **Repair** | Back toward metalled | Capacity recovers |

**A cratered road is not a closed road.** It carries less and it carries slower, and a player who wants
a link *shut* has to either destroy a structure on it or go and fight there. That is the distinction
this design turns on: degradation is cheap, gradual and persistent; closure is expensive and always
costs something specific.

**Which is why bridges are the map author's chokepoints.** A road can only be degraded, so the places
where a link can genuinely be severed are the structures on it, and where those sit is authored. That
is a deliberate lever rather than a side effect — see [§7](#7-what-a-map-author-controls).

**Everyone can repair, and Concord repairs best.** Re-grading is cheaper than grading because the
roadbed survives, so a crater-and-repave exchange costs the attacker credit and Concord *time* — which
is the currency Concord actually cannot spare, and therefore the right one to charge it in. Other
factions repair more slowly and more expensively, because they are not the roadbuilder, but none of
them is locked out: a map that degrades monotonically into unplayability is a map nobody finishes.

**There is no route-wide upgrade tier.** No purchased "all roads are faster". Charter rule 6 puts
technology on the map, and a global percentage appears nowhere; grading has to be a thing that is
watched growing across the terrain, because that is what "its map presence is literally paved" means.

**Grading cannot create freight.** Gate output is fixed, so raising a link's capacity above what the
gate feeds into it does nothing. Concord's curve bends upward until it meets the flow and then flattens,
which is the natural ceiling on paving and means it never needs an artificial one.

This needed an amendment to [ADR 3001](../adr/3001-pathfinding.md) and got one: as first written, its
cost classes could not express a road *cheaper* than plain ground, so grading could only repair mud and
never improve past it — which would have flattened Concord's entire economic identity. Amendments A and
B are accepted and built, so the ladder now runs metalled, graded, plain, mud, rubble, and a class sets
the pace on a cell as well as ranking the route across it.

### 2.3 The map's economic furniture

Three authored site kinds, all neutral and none destructible:

- **Gates.** Points at the map edge where the corridor leaves the map. A gate emits one load every
  *N* ticks, for the whole match. This is the map's total economic output and it is authored, fixed,
  and published — see [§7](#7-what-a-map-author-controls).
- **Yards.** Points along the route where loads accumulate: a weighbridge, a siding, a container
  stack. A yard with nobody working it simply fills up, and a full yard stops accepting, which means an
  unworked corner of the map is *stored* value rather than wasted flow. A player who arrives late at a
  contested yard finds it worth taking.
- **Nodes.** The vertices of the route graph. Crossing one is what Meridian assesses duty on
  ([§2.5](#25-the-three-acquisitions)). Otherwise inert.

Bridges are deliberately not a fourth kind: a bridge is a neutral **structure** standing on a link —
destructible, severable, rebuildable ([§2.6](#26-what-stops-a-link)) — where these three are sites,
fixed and indestructible.

Gates being indestructible and neutral is the valve that prevents the resource-death stalemate. The
flow never stops **permanently** — a link stops while it is being fought over
([§2.6](#26-what-stops-a-link)) and resumes when the fighting does — so a match is
never decided by both players running dry. But the flow is *fixed*, so taking more of it means taking
it from somebody. That is where the pressure comes from, and the fiction supplies it for free: the road
connects two places that are not on the map.

### 2.4 Carriage

A **carrier** is a load capacity with standing orders and no combat value worth mentioning. Given a
yard and a delivery point it runs the round trip by itself, indefinitely, and the player never routes
an individual trip.

"A carrier is a unit" is *nearly* true and the exception matters: AEC does not own carriers at all, it
holds recurring off-map sorties ([§2.5](#25-the-three-acquisitions)). Everything below about distance,
escort and interception applies to a sortie exactly as it does to a truck — what differs is that the
airframe cannot be built, queued, or kept.

That is the whole interaction, and it is deliberately the harvester relationship every working RTS
economy has used, because the decisions it produces are the good ones: *which* yard, *where* the
delivery point goes, *how many* carriers, and *what protects the line between them*. Per-trip
micromanagement produces none of those and costs attention the battle needs.

Income per carrier is therefore
`load value ÷ (travel time out + travel time back + load time + unload time)`,
which makes three things true at once and is the reason the model is shaped this way:

- **Distance is money**, so base placement and expansion are economic decisions rather than tempo ones.
- **Roads are money**, so Concord grading the map is an income increase and not a cosmetic.
- **The line between yard and base is a target**, so map control has a price a player can feel.

### 2.5 The three acquisitions

Every path ends in credit. Every path involves moving something losable. They differ in ceiling, ramp
and risk, and that difference *is* the factions' economic identity.

| | **AEC — the air bridge** | **Concord — ground haulage** | **Meridian — duty on throughput** |
|---|---|---|---|
| Carriage | **Sorties from off-map.** No owned carriers | Owned heavy haulers | Porters, and mostly none at all |
| Route | Map edge → yard → pad. Straight, ignores terrain | Road network, terrain costs apply | — |
| Delivery point | Pad (relocatable, cheap) | Plant (large, permanent) | Permit desk at a node |
| Hard constraint | **Corridor capacity** — simultaneous sortie slots, raised only by holding beacons | Travel time; no count cap | Node coverage, and how busy the map is |
| Fallback | **Contracted ground haulage** | — | Salvage |
| Ramp | Fast | Slowest | Immediate |
| Ceiling | Lowest | Highest | Middling |
| Risk | Highest — carriage and reinforcement share a point of failure | Medium — slow, armoured, long exposure | Lowest — it is not carrying anything |
| Interdiction exposure | Upstream only; its carriage leg uses no links | Upstream **and** on the road it drives | Total — duty *is* throughput |
| Breaks against | Air defence, jamming | Distance, and raids on the road | Being found |

**AEC has no gatherers.** It holds *slots*, not vehicles. A slot is a recurring sortie: it spawns at the
map edge, flies to a yard, lifts a slung load, delivers it to a pad, and exits off-map. Nothing is
queued and nothing is kept. The faction whose force arrives from off-map gets its freight the same way,
through the same doctrine — the alternative, a fleet of owned lift aircraft shuttling back and forth,
is Concord's hauler with different pathing and would make AEC's economy a reskin rather than a
doctrine.

Three consequences:

- **The flight path is the pad's placement.** A sortie enters from the map edge nearest the pad it
  serves, so a forward pad beside a contested yard earns well and transits hostile airspace both ways.
  The interception surface is created by the AEC player's own choices and needs no extra authoring.
- **A downed sortie costs three ways**: the load, which drops where it fell for anyone to recover; the
  slot's downtime while a replacement comes from off-map; and credit for the airframe. That is the
  bible's "every loss is expensive", and it keeps AEC's attrition a *money* problem rather than a queue
  problem.
- **Its economy and its reinforcement are one mechanism.** Pads receive loads and units both, so
  pressure on a pad costs income and army at once. That is how "air denial is near-existential" becomes
  a single mechanic instead of two flavour notes, and it is why AEC's ceiling is deliberately the
  lowest of the three: it is buying speed and reach.

**And a fallback, because no faction may be helpless.** *Contracted ground haulage* is AEC's second
option. The coalition does not carry by road — it **hires**. Trucks, cheap, entirely outside corridor
capacity, paying a **fee per delivered load** rather than a purchase price, so they are an operating
expense with somebody else's margin inside it. That is what a contractor is, and it is why they net
well under the air bridge.

Two properties make them a choice rather than a strictly worse option:

- **They do not consume slots**, so they run *alongside* the air bridge rather than instead of it. A
  player with spare capacity and a safe road runs both.
- **They refuse contested ground.** Not "slower under fire" — a refusal. A route crossing an interdicted
  link is a route they will not accept.

The refusal is what keeps the air dependency's teeth while removing the helplessness: under air denial
AEC degrades into a slower, poorer Concord that also cannot reach anywhere dangerous. It is also where
the bible's contractor fault line — *their incentives are not the coalition's* — stops being backstory
and starts being a thing that happens to the player mid-match.

**Concord's income rises with the map.** A graded road is a cheaper cell class ([ADR
3001](../adr/3001-pathfinding.md) decision 3), so every stretch it paves shortens every future round
trip. Its curve is the only one that bends upward on its own, which is "slow to spin up, then a rolling
wave that does not stop" expressed as arithmetic. The cost is stated in the bible and kept here: **the
road is public.** Everyone moves faster on it, including the people coming for the plant.

**Meridian is paid out of other people's work.** A **permit desk** built at a node assesses a duty on
every load that crosses it — including AEC's and Concord's — and the duty is **additive**. The payer
loses nothing. Nobody's harvester slows down, nobody's yard is locked, and no opponent has homework
imposed on them; Meridian simply gets paid alongside. This is the bible's "greed, not homework", and
the reason it is a hard rule rather than a preference is that the alternative — a faction that taxes
value *out* of an opponent's economy — is a mechanic whose counterplay is chores.

Three consequences, all of them wanted:

- **Meridian profits from its enemies prospering**, and has a standing reason not to strangle a
  logistics line it could kill. That is the Chain's fault line — a war economy that cannot survive
  peace — available as a live decision in a skirmish rather than only as backstory.
- **Meridian can be paid while losing.** It carries nothing, so a lost battle costs it units and no
  income. This is its "attrition of patience", and it is the only faction with that property.
- **Its economy is the one thing it cannot conceal.** A desk becomes visible to the payer once it
  assesses its first load — it serves a notice, which is exactly what the faction would do — so
  Meridian's income reveals its position. Concealment is its defence and its earnings are the leak.
  The faction's two halves are mechanically at odds, which is what the Council and the Chain are.

**Counterplay to a desk is a choice, never a chore.** An opponent may fight for the node, or route
carriage around it and pay in travel time instead. Both are decisions with a price. Neither is
"remember to do a thing every ninety seconds".

### 2.6 What stops a link

Three independent things reduce or stop a link's carrying capacity, and keeping them separate is what
stops the model collapsing into "shoot near the road, road closes":

| | Cause | Shape | Clears when |
|---|---|---|---|
| **Interdiction** | Fighting — harm to an owned asset near the link | Binary: shut or open | The fighting stops |
| **Condition** | Deliberate ordnance or grading ([§2.2](#22-the-three-layers-of-a-supply-chain)) | Graduated: capacity scales with the road | Somebody repairs it |
| **Obstruction** | Wreckage accumulating on the roadway | Graduated, and can reach zero | Wrecks decay, or are cleared |

A battle on a road produces the first immediately and the third afterwards. It never produces the
second.

#### Interdiction

**A link where something is being harmed does not carry freight.**

| | |
|---|---|
| **Trigger** | Damage dealt to a unit or structure **that has an owner**, within the link's radius |
| **Not a trigger** | Fire that harms nothing; damage to neutral scenery, props, wrecks or unclaimed buildings; and damage to the road itself, which is terrain and belongs to nobody |
| **Duration** | Held for *T* after the last qualifying damage, then the link reopens |
| **Effect** | Freight does not traverse the link. Flow accumulates upstream against a cap, and the backlog moves as a surge when the link reopens |

The third exclusion is worth its own line: **shelling a road does not interdict it.** Nothing owned was
harmed, so the link stays open — it simply carries less, because the road is worse. An attack on the
roadway and an attack on the traffic are different attacks with different costs, and a player should be
able to choose between them.

**Harm rather than fire, and the distinction is load-bearing.** Taking a weapon discharge as the trigger
lets a player close a road by shooting a hillside, which is both an exploit and nonsense: a road does
not close because somebody shot a rock. Requiring damage to something *owned* means a link closes only
inside an engagement where somebody's asset is actually being destroyed, so the closure always costs
what it should. Presence alone was rejected for the same family of reasons — one scout parked on a road
forever is a chore, not a decision.

**A defender cannot reopen the road by winning quickly.** Damage to *either* side's assets counts, so a
defender shooting back is also holding the link shut. The road reopens when the fighting stops, not
when it is won. That is the property this mechanic exists for: sustained fighting on a supply line
makes everyone poorer, and both parties know it while they are doing it.

**It makes the middle of the map expensive to fight over.** Central links feed both players, so a
positional grind on the midline is mutual economic destruction. The efficient economic attack is a
**deep raid** at a link that feeds only the enemy — near their gate, behind their line. One rule, and
the game is pushed away from a static front and toward raiding.

**It differentiates the factions with no special-casing**, which is the sign the rule is the right
shape. AEC's sorties traverse no links, so its carriage leg is immune — but a yard that was never fed
is still empty, so it is exposed upstream like everybody else. Concord is hit twice: the flow into its
yards stops *and* the fight is happening on the road its haulers are driving. Meridian is hit hardest
of the three, and in a way that turns out to be the best thing in this design: duty falls as fighting
rises while salvage rises with it, so its two wings end up on
[opposite sides of the war's intensity](faction-mechanics.md#the-council-and-the-chain-sit-on-opposite-sides-of-the-war).

**A link with nothing on it cannot be closed**, and that corollary is fine. An empty road held by
raiders with nobody to shoot keeps flowing. In practice it is not the case that arises: what a deep
raid finds on a supply line is *carriers*, and a carrier is an owned object whose destruction both
takes the load and shuts the link. The raid's target and the interdiction trigger are the same object.

**This is not a breach of the rule against gating an opponent's economy.** That rider constrains
Meridian's *duty*, which must never be extractive. Interdiction is combat — available to all three
equally, and paid for in the only currency this design accepts for an economic attack, which is a
fight.

The radius, the hold duration and the upstream cap are
[measured, not chosen](balance.md#54-numbers-that-are-measured-not-chosen).

#### Obstruction

**Enough wreckage on a roadway blocks it**, and this needs no mechanism of its own. A wreck
([§3.3](#33-wrecks)) stamps a dearer cost class on the cells it covers, so one wreck is an
inconvenience and a road full of them is impassable in aggregate. The link's capacity derives from the
cheapest route along it, so it falls as the wreckage builds and reaches zero when nothing can get
through.

A wreck stamps a **cost class rather than a footprint**, deliberately. A footprint is a hard wall, and
a single dead truck should not be one — you push past it slowly. What closes a road is the
accumulation, which is the thing that should be true: a sustained battle on the corridor eventually
chokes it with its own casualties, long after the shooting has moved on.

This is the delayed, physical half of the property interdiction gives immediately. Fighting on a supply
line costs everyone twice — once while it happens, and again for as long as the debris sits there.

**And it hands Meridian something nobody designed.** Recovering wrecks is how Meridian builds its army,
and recovering wrecks is also how a blocked road gets cleared. Its salvage crews are its road-clearing
crews, so the faction that lives on throughput is paid to restore the throughput a battle destroyed.
Nothing was added to make that true.

#### Severance, and the detour it forces

**The route graph is a graph, so flow reroutes.** When a link's capacity reaches zero, freight takes the
next-best path through the graph if one exists — longer, and therefore slower and lower-capacity — and
only accumulates upstream when there is no path at all. Carriage does the same thing for the same
reason: a hauler paths over terrain, so a severed crossing is a longer drive rather than a stopped one.

That single property is what makes **demolishing a bridge a different act from closing a road.**
Interdiction and cratering reduce what a route carries. Severance *moves* the route.

**A bridge is a neutral structure and destroying it does not interdict anything** — nothing owned was
harmed, so no link shuts on the fighting rule. It severs, and the traffic goes the other way. The two
mechanisms are cleanly separate and they compose.

##### Shaping

The composition is the interesting part, and it is a plan rather than a reaction:

1. Work out where the traffic goes if a crossing is gone.
2. Take that ground, and fortify it.
3. **Then** drop the bridge.

The detour is not a guess — it is the graph's next-best path, so it is knowable in advance. A player who
has read the map can decide where an opponent's supply will be *before* forcing it there, and meet it
with something already dug in. Cheaper than hunting convoys across the whole corridor, and it turns a
demolition into an ambush with the ambush set up first.

**AEC's lexicon already had the word for this.** The bible gives it *shaping operations* among its
euphemisms, and this is what the phrase would mean if it meant anything: you do not attack the enemy's
logistics, you rearrange the ground until the logistics come to you. The faction that talks like a
staff college gets the mechanic its vocabulary was already describing.

Three answers exist, so it is a decision and not a trap: rebuild the crossing, clear the outpost, or
accept the longer route and pay in income. **This is also the best argument for having kept the three
mechanisms separate** — severance, occupation and interdiction chain into one operation precisely
because they are three different things. One merged "the road is damaged" quantity could not have
produced it.

##### Rebuilding a crossing, and interrupting one

**A crossing can always be rebuilt, and it is deliberately expensive and slow.** Any faction with an
engineering capability can do it; Concord does it fastest and cheapest, because it is the roadbuilder.
The cost has to be real, or demolition is a nuisance rather than a decision, and the time has to be
real, or an outpost overlooking the site is watching nothing happen.

**A rebuild is a build site, so it obeys [§4.2](#42-construction) and needs no new rule.** From the
moment work starts there is an object standing at the crossing, and everything follows from that:

- **Anyone can interrupt it.** Kill the engineer and work stops where it stopped. Progress already
  invested stays on the site, so an interruption costs the *tempo*, not the investment, and the owner
  can come back and resume.
- **Destroying the site is the harsher answer**, and it costs what it should: cancelling a build refunds
  the unspent remainder, and having it destroyed refunds nothing. An opponent who wants the crossing
  gone permanently has to keep coming back and levelling the works, which is a commitment of its own.
- **So a crossing becomes a place, not an event.** It is contested repeatedly over a match, and whoever
  holds the ground around it decides whether the bridge exists at all.

That last line is what makes the outpost from the previous section worth more than one ambush. It
covers the detour *and* denies the repair, so a player who takes the high ground beside a crossing has
bought both halves of the argument. And the counter is equally clear: the crossing's owner has to
contest that ground before an engineer is worth sending, which is a fight over terrain that neither
side chose for its own sake — which is the kind of fight a corridor should generate.

### 2.7 Why expansion is a real decision

The M6 charter asks for "a rate that makes expansion a real decision", so it is worth stating exactly
what makes it one here. Income is `load value ÷ round-trip time`, gate output is fixed, and yards
store what nobody collects. An expansion therefore buys a shorter round trip on flow somebody else
would otherwise take, at the cost of a structure to defend and a line to hold. The break-even is
computable, it moves with the map, and the answer is different in the first four minutes than in the
fourteenth. That is the decision. See [balance.md §5](balance.md#5-economic-benchmarks) for the
numbers that set where the break-even lands.

### 2.8 Deliberately not a second currency

Power, fuel, ammunition and manpower were all considered as a second harvested resource and all
rejected in favour of **faction-specific structural constraints** — corridor capacity, queue capacity,
standing convertible buildings. The reasoning is that a second global currency makes all three
factions more similar (everybody gathers two things) while a structural constraint makes them less so,
and the bible has already specified three different constraints. A second currency also doubles the
balance surface for one axis of decision.

The one candidate still open is **the grid**: the bible gives Concord substations and Meridian
"illegal grid taps and generators", which is a map-level utility with three obvious asymmetric uses
rather than a currency. It is designed and not scheduled — see
[§8](#8-designed-and-not-scheduled).

---

## 3. Combat

### 3.1 The model

Health, damage, range and cooldown are **integers**, and every rate is integer arithmetic per tick.
This is not a stylistic preference: the kernel is almost entirely integer by construction ([ADR
0007](../adr/0007-simulation-arithmetic.md)), and an economy or a damage model that accumulates
fractions is a determinism liability that buys precision nothing needs. Where a rate does not divide
evenly, it accumulates as an integer numerator over a fixed denominator with the remainder retained —
exact, driftless, and hashable.

Resolution per shot: range check, cooldown check, seeded accuracy roll from a named stream, damage
lookup, integer subtract. Nothing else. No physics decides anything ([ADR
0008](../adr/0008-physics-engine.md) — the engine is *told* the answer).

One addition is expected, recorded here so "nothing else" stays honest: **area of effect is an open
decision.** The table below describes blast as artillery's damage type and frag as airburst — both
name area weapons — while the list above resolves a shot against a single target. Whether a shot
gains an area lookup (more integer reads inside the same resolution, never physics) is settled with
combat's first pass, not silently by it.

### 3.2 Damage types and armour classes

Four damage types against five armour classes, as integer percentages:

| | infantry | light | heavy | structure | air |
|---|---|---|---|---|---|
| **kinetic** — autocannon, small arms | 100 | **150** | 50 | 50 | 100 |
| **heat** — shaped charge, guided missile | 50 | 125 | **150** | 75 | 50 |
| **blast** — high explosive, artillery | 150 | 100 | 75 | **150** | 50 |
| **frag** — airburst, anti-personnel | **200** | 75 | 50 | 75 | 125 |

Every armour class has exactly one clear nemesis and one clear resistance, and every damage type has a
best and a worst target, so a player can hold the whole table in their head after two matches.

**No multiplier is zero, and a test asserts it.** Nothing in this game is immune to anything. That is
the bible's constraint — "no faction is allowed to be characterised as helpless against something" —
arriving as a number, and it is what keeps a counter from becoming a hard gate: an infantry squad
against a heavy tank is a bad trade, not a null one, and a player who guessed the composition wrong
loses ground rather than losing the ability to act.

Electronic warfare is **not** in this table. Jamming, spoofing and GPS denial do no health damage;
they suppress capability — a lift returning empty, a sweep returning nothing, a guided weapon reverting
to unguided. Modelling them as damage would make them a fifth column here and would let them kill,
which is not what they do.

### 3.3 Wrecks

A destroyed vehicle or structure leaves a **wreck object** in the simulation with a decay timer.
Wrecks matter mechanically for four reasons and are therefore simulation state rather than
presentation:

- **Meridian recovers them** into degraded units
  ([faction-mechanics.md](faction-mechanics.md#army-captured-not-built)).
- **They obstruct.** A wreck stamps a dearer **cost class** on the cells it covers — not a footprint,
  because a single dead truck is something you push past rather than a wall. The accumulation is what
  closes a road ([§2.6](#26-what-stops-a-link)).
- **Rubble is a cost class too.** A collapsed building grades the ground it fell on, which ADR 3001
  already anticipates.
- **A battlefield reads as one.** The tumble is presentation and may differ between clients; the wreck
  that is there to be recovered may not.

---

## 4. Production and construction

### 4.1 The shared frame

Every faction produces through the same five quantities — **cost, lead time, prerequisite, delivery
point, queue** — and diverges in what those quantities are worth to it. That the frame is shared is
what makes the divergence legible and what makes balance possible; a faction with a bespoke production
*grammar* cannot be compared to one without.

### 4.2 Construction

A structure is placed, validated, and built on site. Placement validity is footprint clearance,
terrain passability, and proximity to whatever the structure requires. A build site is a real object
from the moment it is placed: it can be attacked, and cancelling refunds the unspent remainder and
nothing more.

### 4.3 Technology

Charter rule 6 in force: every tier a player advances is a structure standing on the map. The three
expressions are in [faction-mechanics.md](faction-mechanics.md); the shared rule is that **no
capability is ever purely a number in a player's account.** If a player has it, an opponent can find
the thing that grants it.

### 4.4 The ceiling is built, not granted

Whatever bounds an army is a structure count:

- **AEC** — pads and beacons. Delivery throughput and corridor capacity, together.
- **Concord** — factories and their queue slots.
- **Meridian** — converted buildings still standing.

The immediate consequence, and the reason this is a charter rule: **an opponent can lower your
ceiling.** A global pool cannot be attacked, so it turns every fight into a question about the front
line only. A built ceiling means a raid twenty minutes deep into the map can shrink what its victim is
allowed to field, which is the kind of decision an RTS map exists to host.

---

## 5. Vision, fog and detection

Vision lives in the simulation, not the renderer, and M6 already states why: what a player can see
determines what their units may target. Three states per object per player:

| State | Meaning |
|---|---|
| **Shrouded** | Never seen. Terrain unknown. |
| **Fogged** | Seen before, not now. Last known state remembered; a remembered object may be gone. |
| **Observed** | In vision of something of the player's, now. |

**Detection is a fourth axis, not a fourth state.** A concealed object inside a player's vision is
*not* observed until something detects it or it reveals itself by firing. This is what makes AEC's
purchased vision and Meridian's purchased ambiguity oppose each other directly rather than merely
differ — one faction spends credits to move objects from fogged to observed and the other spends them
to keep objects undetected inside enemy vision. Two mechanics on one axis, pointed in opposite
directions, is worth more than two unrelated gimmicks.

Remembered state is per player and is allowed to be wrong. A fogged structure that has been destroyed
still appears until somebody looks. That is a feature — it is where a feint pays.

---

## 6. Victory

Two conditions, either sufficient:

- **Corridor control.** Hold *N* of the map's route nodes continuously for *T*. Authored per map.
- **Annihilation.** No production capability and no army remaining.

Control is the primary condition because the setting demands it: nobody here is fighting for
territory, so a game whose only ending is extermination contradicts its own premise. It also removes
the turtle stalemate — a player who declines to leave their base loses on a clock rather than
outlasting the match.

Annihilation stays because control victories need a floor. **Open question:** whether a control
timer that has started should be visible to the losing player. Hiding it is more honest to the
setting; showing it is the only version a player can respond to. Leaning toward showing it, on the
grounds that an unannounced loss condition is indistinguishable from a bug.

---

## 7. What a map author controls

The economy is authored, which means a map is a balance surface and has to be treated as one. Five
knobs, and all five are **published in the map's own metadata** so a player can read them before
choosing to play it. [ADR 3002](../adr/3002-corridor-economy.md) decision 14 names the first four;
the bridges row is this document extending the record's list:

| Knob | Effect | Failure if wrong |
|---|---|---|
| Gate count and rate | Total economic output of the map | Too high and position stops mattering; too low and the match is a knife fight |
| Yard count and placement | Where income has to be defended | Clustered yards make one fight decide everything |
| Route node count | Meridian's income ceiling | Node-dense maps are a Meridian map, silently |
| Convertible buildings | Meridian's production ceiling | A map with no town is a map Meridian cannot play |
| **Bridges and their alternate paths** | Where a route can be *severed*, and where the traffic goes when it is | A graph with no redundancy turns one demolition into a win; one with too much makes severance pointless |

The last row is the one that rewards the most thought, because it is the only knob that authors a
*plan* rather than a quantity. Every crossing is a place an opponent can move somebody's supply to
ground of their choosing ([§2.6](#26-what-stops-a-link)), so where the detours run is where the map's
fights will be. A corridor with exactly one alternate path per crossing is a map with legible, arguable
chokepoints; a lattice has none, and a single chain is a map decided by whoever blows the first bridge.

The third and fourth rows are why per-map balance cannot be an afterthought: both of Meridian's
ceilings — income, from route nodes, and production, from convertible buildings — are set by map
authoring rather than by its own build order. A tournament map set has
to state these figures, and a stock map that departs from the reference values has to say so.

---

## 8. Designed and not scheduled

Recorded so none of it reads as forgotten:

- **The grid.** Substations as capturable map utilities, with three asymmetric uses — Concord's plants
  scale throughput with grid access, Meridian taps it illegally for free at the cost of a detectable
  signature, AEC ships generators and pays for them in corridor capacity. This is the most promising
  open item because it hangs three faction-characteristic behaviours off one authored map feature.
- **Weather and the air bridge.** The renderer has blendable weather; an air economy that degrades in
  low cloud is nearly free and would make an existing presentation system a strategic input.
- **Contributing-nation contingents** as an AEC mechanic — national caveats as a real restriction on
  what a given contingent will do. Excellent character, unclear whether it is fun. Contracted haulage
  refusing contested ground ([§2.5](#25-the-three-acquisitions)) is the same idea at a smaller scale
  and is the cheap test of whether it plays well.
- **Civilian traffic as simulation objects.** The corridor's traffic is presentation today
  ([§2.2](#22-the-three-layers-of-a-supply-chain)) and could be promoted to real objects, at which
  point destroying commerce to starve Meridian's duty becomes a strategy. That is a legitimate move in
  this setting rather than a gimmick, but it is a decision about civilian death and gets made
  deliberately or not at all.

## 9. Explicitly not in this document

- **Numbers.** Costs, healths, rates and multipliers other than the damage table live in
  [balance.md](balance.md), which also owns the method for setting them. The one exception is the
  damage table, which is here because its *shape* — four by five, no zeros — is a rule rather than a
  tuning value.
- **Per-faction detail.** [faction-mechanics.md](faction-mechanics.md).
- **Campaign and mission structure.** Scripted missions are a different subject and the bible's game
  split governs them.
- **Multiplayer format.** Team sizes, seat counts and modes wait on a playable one-versus-one.

---

## 10. What this document obliges the engine to gain

The design README claims the faction bible is a source of engine requirements and has already been
one. This document is the same claim continued, and the list is short enough to state:

| Requirement | Where it lands | Already promised? |
|---|---|---|
| Scenario `routes`, `gates`, `yards` | [scenario.md](../formats/scenario.md) | No — additive fields, needs a format decision |
| Template `health`, `cost`, `build_time`, `weapons`, `armour_class`, `vision`, `capacity` | [templates.md](../formats/templates.md) | Yes — "each arrives with the M6 mechanic that reads it" |
| Template `footprint`, `passage` | same | Yes — built, [ADR 3001](../adr/3001-pathfinding.md) decision 4 |
| Carrier round trips as standing orders | `cic_sim` | Partly — standing orders exist |
| Off-map sortie slots with recovery timers | `cic_sim` | No |
| Integer credit accumulation with retained remainder | `cic_sim` | No |
| Per-link interdiction state, driven by damage events | `cic_sim` | No |
| Upstream backlog with a cap, and the reopening surge | `cic_sim` | No |
| Flow rerouting over the graph when a link is severed | `cic_sim` | No |
| Destructible neutral crossings, and rebuilding them | `cic_sim`, templates | Partly — `passage` is built and grants the crossing; destroying and rebuilding one is not |
| Wreck objects with decay, stamping a cost class | `cic_sim` | Partly — ADR 3001 amendment C is accepted and the stamping mechanism is built; the wreck waits on combat |
| Link capacity derived from road condition and obstruction | `cic_sim` | No |
| Convertible neutral structures | `cic_sim`, templates | No |
| Detection as an axis beside vision | `cic_sim` fog | No |
| Runtime cell-cost edits for grading, cratering and repair | `cic_sim` pathfinding | Yes — built, ADR 3001 decisions 4 and 7; what edits the cells is not |
| A cell's cost class reaching **movement rate**, not only path ranking | `cic_sim` units | Yes — built, ADR 3001 amendment B |
| Cost classes that can express better-than-ground | ADR 3001 | Yes — built, ADR 3001 amendment A |
| Cosmetic corridor traffic | `cic-render` | No — presentation only, no simulation state |
| Per-faction display strings for one currency | string table | Yes — exists |
| Per-instance tint for recovered hulls | `cic-render` | Yes — exists |

<!--count:promised-->Ten<!--/count--> of <!--count:total-->twenty<!--/count--> are already promised
or built, which is the argument for writing this now rather than after M6's economy line is
implemented: the cheap half of the list is cheap *because* the bible was written before the renderer.
<!--count:amendments-->Three<!--/count--> of the <!--count:total-->twenty<!--/count--> are amendments to
an accepted record rather than new work, and all of those were found by writing this document rather
than by building anything — which is the other argument for the order.

*The three counts in that paragraph are generated from the table above by
`tools/generate-doc-counts.py`, and CI fails on a stale one. They were each wrong at least once while
this document was being written, because adding a row here does not remind anybody to edit a sentence
in two other files.*
