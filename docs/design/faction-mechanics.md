# Faction Mechanical Identity

**Status:** working draft, downstream of [ADR 3002](../adr/3002-corridor-economy.md)'s review.
**Scope:** how each faction expresses the shared rules in [mechanics.md](mechanics.md). One section
per faction, in the same order the bible uses.

**This document is derived, not invented.** Every entry below traces to a doctrine line in
[faction-bible.md](faction-bible.md). Where the bible states a strength or a weakness, the job here is
to find the mechanic that *is* that property rather than a mechanic that gestures at it. A faction
feature that does not trace to the bible is either a bible amendment or a mistake, and it has to be
declared as one.

---

## 1. How to read the sections

Each faction gets nine fields, in this order:

| Field | Question it answers |
|---|---|
| **Economy** | How it turns freight into credit |
| **Production** | Where units come from |
| **Technology** | How it advances, and what an enemy destroys to stop it |
| **Information** | What it knows and what it hides |
| **Defence** | What keeps it alive |
| **Army character** | What its force feels like to command |
| **Signature** | The one thing only it can do |
| **Failure mode** | How this faction gets designed badly, stated so it can be watched for |
| **Counterplay** | What an opponent does about it |

The **failure mode** field exists because the bible has one for character and mechanics need the same
guard. Each faction has a specific way of drifting into being either the obvious best pick or a
gimmick, and the drift is always toward its own fantasy.

---

## 2. The three at a glance

| | **AEC** | **Concord** | **Meridian** |
|---|---|---|---|
| Buys | vision | permanence | ambiguity |
| Income ramp | fast | slowest | immediate |
| Income ceiling | lowest | highest | middling |
| Income risk | highest | medium | lowest |
| Carriage | sorties it does not own | haulers it owns | none, plus porters |
| Force arrives | from off-map, anywhere it has a pad | in batches, from few places | from the map itself |
| Army ceiling set by | pads and beacons | factories and queue slots | buildings left standing |
| Tempo | continuous, reactive | lumpy, committed | continuous, cheap |
| Hurt by interdiction | upstream only | upstream and on the road | totally, and rewarded for it too |
| Loses to | air denial, jamming | anything unplanned | being observed and pinned |
| Wants the map | open and quiet | paved | busy — and, in its other half, bloody |

**The last row is the one worth internalising.** All three want the road open — that is the bible's
premise and it is preserved — but they want *different traffic* on it, and that is where a match's
shape comes from. AEC wants an empty corridor it can fly over. Concord wants a graded one. Meridian
wants a crowded one, including its enemies' crowd — and its other wing wants a fought-over one, which
is [the thing this design is proudest of](#the-council-and-the-chain-sit-on-opposite-sides-of-the-war).

---

## 3. Rules that apply to all three

Stated once so no section has to repeat them:

- **Every faction covers every tactical role.** Bible constraint, not a balance preference. Direct
  fire, indirect fire, anti-armour, anti-air, anti-infantry, reconnaissance, engineering, transport,
  logistics: all present in all three. Divergence is in *how* a role is filled and never in whether it
  exists.
- **No faction has a role it fills badly enough to be helpless.** A weakness is a worse trade, never a
  null one — the no-zero rule in the damage table, applied at the level of the roster.
- **Doctrine lives in the economy and the production layer.** Unit statlines are where balance is
  *achieved*, not where character is expressed. A faction that needs a 40% better tank to feel
  different has been designed wrong; see [balance.md §4.2](balance.md#42-the-asymmetry-budget).
- **Every faction's economic structures are visible to a determined opponent.** The three are found by
  different means and at different cost, but none is unfindable.

---

## 4. AEC — Allied Expeditionary Command

> *Production as logistics. Force arrives from off-map. Vision is purchased.*

### Economy

**The air bridge, and no gatherers at all.** AEC holds *slots*, not vehicles. A slot is a recurring
sortie: it spawns at the map edge, flies to a yard, lifts a slung load, delivers it to a **pad**, and
exits off-map. Nothing is queued, nothing is owned, nothing can be kept. The faction whose force
arrives from off-map gets its freight the same way, through the same doctrine — which is the whole
point, because a fleet of owned lift helicopters shuttling back and forth is Concord's hauler with
different pathing and would make AEC's economy a reskin.

Capped hard by **corridor capacity**: how many sorties may be airborne at once, raised only by holding
**beacons**. Beacons are cheap, quick, placeable anywhere including deep forward, and trivially
destroyed. So AEC's income is a *number of small things it must hold*, spread across the map rather
than concentrated at home.

**Pad placement is the flight path.** A sortie enters from the map edge nearest the pad it serves, so a
forward pad beside a contested yard earns well and crosses hostile airspace twice per trip. The
interception surface is drawn by the AEC player's own choices, which is why it needs no authoring and
why it moves as the front does.

**A downed sortie costs three ways**: the load, which drops where it fell for anyone to pick up; the
slot's downtime while a replacement comes from off-map; and credit for the airframe. AEC's attrition is
a money problem rather than a queue problem, and that is the correct pairing for the lowest income
ceiling in the game.

**Its economy and its reinforcement are one mechanism.** A pad receives loads and units both. Pressure
on a pad costs income and army at the same time, and that single fact is the bible's "air denial is
near-existential" — not a modifier, a shared point of failure.

**Interdiction barely touches its carriage and fully touches its supply.** Sorties traverse no route
links, so a fight on the road does not stop AEC's lift. It also does not conjure freight into a yard
that was never fed, so AEC is exposed upstream exactly like everyone else. It is the faction that can
operate over a broken road and still cannot invent what never arrived.

### Fallback: contracted haulage

The coalition does not carry by road. It **hires** — and this is where the bible's contractor fault
line stops being backstory.

Trucks, cheap, entirely outside corridor capacity, paid by a **fee per delivered load** rather than
bought, so they are an operating expense with somebody else's margin inside it. They net well under the
air bridge and they always will; that is not a tuning target, it is what a contractor is.

Two properties make them a real choice rather than a strictly worse option:

- **They cost no slots**, so they run *alongside* the air bridge. A player with spare capacity and a
  safe road runs both, which is the ordinary case rather than the desperate one.
- **They refuse contested ground.** Not slower under fire — a refusal. A route crossing an interdicted
  link is a route they will not accept, and no order overrides it.

The refusal is what keeps the air dependency's teeth while removing the helplessness the bible forbids.
Under sustained air denial AEC does not die; it degrades into a slower, poorer Concord that also cannot
reach anywhere dangerous. That is the right punishment, and the player chose it.

### Production

No factories. A unit is paid for and **arrives from off-map** at any pad the player owns, after a lead
time. Pads are cheap, fast to establish, relocatable, and thin-footprinted.

The consequence is AEC's central advantage: **it reinforces forward.** A pad established behind a
push delivers the next wave to the front rather than to home, so AEC does not pay the walk. It is the
only faction that can move its production geography during a match, and losing air superiority takes
that away rather than merely making it worse.

### Technology

**Authorisations**, purchased at a command post. No build time — a tier is granted the moment it is
paid for, which is a legal permission rather than a construction project, and reads exactly as this
faction should. The command post is the single point of failure and is deliberately not cheap to
replace: AEC's tech is the fastest to acquire and the most concentrated to defend. Whoever certifies
holds real power, which the bible already says about this faction's internals.

### Information

**Purchased vision.** ISR sweeps and satellite windows on cooldown reveal an area for a duration. The
cooldown is a second, non-transferable economy — it cannot be bought with credit, only spent or
wasted — so AEC habitually fights with better information than anyone else and has to decide *when*
to know things.

**Jamming is what breaks this, and it must break it properly.** Under electronic attack: sweeps return
nothing, guided weapons revert to unguided, and lifts return empty. Not a percentage penalty — a
capability withdrawn. The bible says the whole apparatus degrades badly under EW and GPS denial, and a
10% accuracy debuff is not that.

### Defence

Thin. Repair is drone-mediated, fast, and paid in credit rather than time — AEC's attrition is a
*money* problem, which is why its lowest income ceiling is the correct pairing. Static defence is
minimal and temporary, in keeping with a camp that could be gone in seventy-two hours.

### Army character

Small, exact, expensive, and always slightly too few. Every engagement is worth withdrawing from. The
player who plays AEC as a mass army loses to Concord and should.

### Signature

**Deliver a force where nobody expected one, at a beacon nobody found.** Everything above serves that
sentence.

### Failure mode

**AEC becomes the correct choice at every skill level**, because precision, information and mobility
are exactly the things a good player converts into advantage. The three guards, all of them
structural rather than statistical: the lowest income ceiling of the three, repair paid in credit, and
a ceiling composed of small holdings spread across a map that an opponent can pick off one at a time.

**The contractors become the main economy.** The fallback exists so air denial degrades AEC instead of
killing it; if it is ever cost-competitive with the air bridge, AEC plays the whole match as a worse
Concord and its doctrine evaporates. Two structural guards, neither of them a tuning number: the fee
per load means somebody else's margin is always inside the income, and the refusal to enter contested
ground means contractors are unavailable in precisely the situations that decide matches.

And the third — **AEC reading as the good faction**, because clean precision is aesthetically
sympathetic. Mechanically, its ability to strike anywhere on a mandate is the least accountable
capability in the game.

### Counterplay

Field air defence, which every faction has. Jam it. Hunt beacons rather than pads — pads come back and
capacity does not. And force it to trade: it wins a fight it chose and loses a war of exchanges.

Against a grounded AEC falling back on contractors, the answer is **interdiction**, and the two
mechanics compose into a clean triangle: air defence beats the air bridge, interdiction beats the
contractors, and the air bridge beats interdiction by flying over it. There is no single investment
that shuts AEC's economy down — which is the correct amount of pressure to be able to apply to it.

---

## 5. Concord — Continental Concord

> *Production as industry. Builds permanent infrastructure outward. Its map presence is literally paved.*

### Economy

**Ground haulage.** Heavy haulers drive loads from yards to a **plant** along the route, subject to
terrain cost. Slow, armoured, high capacity per trip, and long-exposed.

**Its income rises with the map.** A graded road is a cheaper cell class, so every stretch Concord
paves shortens every future round trip it makes. It is the only faction whose income curve bends upward
without new territory, which is the bible's "slow to spin up, then a rolling wave that does not stop"
as arithmetic rather than as adjective. No count cap: the highest ceiling of the three, reached last.

**Grading is spatial work, not a purchase.** An engineering unit builds along a drawn path, at a cost,
over time. There is no route-wide upgrade tier and there must not be: a global "all roads are faster"
is a menu entry that appears nowhere on the map, and this faction's map presence being *literally
paved* means the paving has to be a thing you watch grow.

**The road is public**, and this stays. Everyone moves faster on it, including whoever is coming for
the plant. Concord's own investment shortens its enemies' approach — an argument against paving that
the faction is nevertheless correct to ignore, and a mechanic that carries "everybody wants the road
open" without a line of dialogue.

**And the road can be broken.** Cratering is available to all three, cheaply to AEC, and it is the
direct counter to the only rising income curve in the game. Re-grading is cheaper than grading because
the roadbed survives, so the exchange costs Concord *time* rather than credit — which is the currency
it actually cannot spare, and therefore the right one to charge it in.

**Interdiction hits Concord twice.** A fight on its haul route stops the flow into its yards *and*
threatens the haulers driving it. It is the most road-dependent faction in a design where the road can
be closed by fighting on it, and it has no leg of its economy that escapes.

### Production

**Batch delivery.** Few, very large factories with parallel queues. A batch is paid for as a batch and
arrives *all at once* — a platoon, not a unit. Longer lead time than either rival, better cost per
unit.

The tempo consequence is the whole faction: Concord's force does not trickle, it lands. Its pushes are
committed because they were committed several minutes before they left. **Killing a factory mid-batch
is the most damaging single strike available against any faction in the game**, which is the correct
price for the best cost efficiency.

### Technology

**Plant upgrades.** Long build, permanent, physically visible, and the tier is the structure. Slowest
tech in the game and the hardest to remove once standing. Where AEC's advance is a signature, Concord's
is poured.

### Information

**The worst of the three**, deliberately. It buys permanence, not vision. Static sensor structures and
observation posts rather than sweeps: Concord *knows the ground it holds* extremely well and knows very
little else. Against Meridian this is its hardest matchup and should be — the faction with the least
detection against the faction that sells ambiguity.

### Defence

The best in the game, and static. Depots that repair and reinforce forward, hardened positions, real
fortification. Concord holds what it has taken better than anyone and takes it slowest.

### Army character

Mass with attrition tolerance. Losses are affordable; time is not. The Concord player thinks in waves
and is punished for improvising.

### Signature

**A push that was decided four minutes ago arriving intact, on a road it built, into a position it can
immediately hold.**

### Failure mode

**Concord becomes a snowball.** Rising income, the highest ceiling, the best fortification and the best
cost efficiency compose into a faction that cannot be stopped once it is ahead. The counterweight is
its turning radius and it has to be *real*: batch commitment must be genuinely irreversible, lead times
long enough that a wrong guess costs a full cycle, and its information deficit severe enough that wrong
guesses actually happen. The second failure mode is the mirror: **Concord as an unplayable slog**, where
the ramp is so long that the first ten minutes contain no decisions. The fix for that is never a faster
ramp — it is decisions during the ramp, which is what road routing and yard selection are for.

### Counterplay

Do not fight the wave; be somewhere else when it lands. Raid the haulage line, which is slow and far
from home. Use its own roads to arrive faster than it planned for. And attack the schedule — a factory
struck mid-batch costs more than the units it was building.

---

## 6. Meridian — Meridian Council

> *Production as occupation. Economy as taxed throughput. Army captured, not built. Defence by concealment.*

Meridian is the faction most likely to be got wrong, because three of its four doctrines are
mechanically unusual and the temptation is to soften them into ordinary ones. The bible's craft note
governs here: *lead with the bureaucracy; the violence is the enforcement arm of a filing system.*

### Economy

**Duty on throughput.** A **permit desk** built at a route node assesses a duty on every load that
crosses it, its own and its enemies' alike. The duty is **additive** — the payer loses nothing, no
opponent's economy slows, and no opponent is handed a chore. Meridian also runs its own modest
collection: porters, many and cheap and individually trivial, so raiding them costs a raider more
attention than it takes from Meridian.

The three properties this produces are all deliberate:

- **It profits from its enemies prospering.** Killing a logistics line it could tax is a real loss, so
  Meridian holds a decision nobody else has, and it is the Chain's fault line made playable.
- **It can be paid while losing.** It carries nothing; a lost battle costs units and no income.
- **Its income reveals it.** A desk becomes visible to the payer once it assesses its first load — it
  serves a notice. Concealment is Meridian's defence, and its earnings are the one thing it cannot
  conceal. The Council and the Chain are mechanically at odds, which is what they are.

#### The Council and the Chain sit on opposite sides of the war

This is the best property in the design and it was not designed — it fell out of
[interdiction](mechanics.md#26-interdiction-a-contested-link-stops) meeting an economy that was already
written.

Duty is charged on throughput, and an interdicted link carries none. So **every fight anywhere in the
network Meridian taxes cuts Meridian's income**, whoever is fighting and whoever wins. Salvage runs the
other way: the more fighting there is, the more hulls there are on the ground to recover.

Meridian therefore has two income sources that move in **opposite directions along the same axis** —
the intensity of the war:

| | Earns when | Wants |
|---|---|---|
| **The Council** — duty | The corridor is quiet and busy | The war fought somewhere else |
| **The Chain** — salvage | The corridor is a battlefield | The war fought here |

That is the faction's central contradiction — *the Council wants recognition and understands
recognition means stopping; the Chain is the war economy and cannot survive peace* — expressed as
arithmetic a player feels in their income, rather than as a line in a briefing. A Meridian player
choosing where to commit is choosing which wing of their own faction to fund, every time, without the
game ever saying so.

It also does the balance work: the faction most damaged by a busy map is the same faction most rewarded
by it, so neither a quiet game nor a bloodbath is simply Meridian's best case. Nothing had to be added
to achieve that.

### Production

**Conversion.** Meridian builds no factories. It converts a standing map building — a garage, a silo,
an apartment block, a workshop — into production, with output determined by what the building is.

This is the mechanic that reaches furthest outside its own faction, exactly as the bible intends: it
forces the other two into the question of **whether to level a town to kill a drone shop**, and the
answer is never comfortable. It also means Meridian's production geography is authored by the map and
edited by the war, so its ceiling falls as the map is destroyed. A player who has spent thirty minutes
demolishing a district has been beating Meridian the whole time and may not have noticed.

### Technology

**Recovered patterns.** A tech option becomes available only after Meridian has salvaged an example of
it. Its tech tree is *its enemies' tree*, one step behind and degraded, and it cannot be advanced
further than what the war has handed it.

The strategic shape of that is unlike anything the other two have: Meridian's power curve is a function
of how much fighting has happened, not of how long the match has run.

### Army: captured, not built

**Salvage is doctrine.** Meridian recovers wrecks into working units — degraded reliability, lower
health, occasional breakdown, cheaper than the original, wearing the original silhouette under a coat
of Meridian yellow with the previous markings showing at the panel edges.

Four rules keep it from being a snowball, and they are all necessary:

1. **Only Meridian salvages.** Other factions clear wrecks; they do not gain from them.
2. **Recovery costs credit and time and needs a recovery vehicle**, which is slow, unarmed, and has to
   stand on the ground where the battle was.
3. **Recovered units cannot be upgraded** and cannot exceed the pattern they came from.
4. **Wrecks decay.** The field is a window, not a bank.

**The anti-snowball property is the interesting one:** Meridian gains from *large battles regardless of
who won them*. It can lose a fight, hold the field afterwards, and come out with material. That is a
comeback mechanism which pays for aggression by both opponents rather than punishing the loser further,
and it is precisely the bible's "attrition of patience".

**The misidentification engine is a real mechanic, not a paint job.** A recovered hull reads at a
glance as the faction it was built by. In a three-way match that is genuine target confusion, and in a
two-way one it is a moment of hesitation that costs a second. Per-instance model tint already exists in
the renderer for exactly this.

### Information and defence

**Purchased ambiguity**, the direct opposite of AEC's purchased vision and on the same axis. Tunnels,
camouflage, false signatures that appear on an enemy sweep, illegal grid taps. A concealed unit inside
enemy vision is not observed until it fires or is detected ([mechanics.md
§5](mechanics.md#5-vision-fog-and-detection)).

Terrifying while unobserved and brittle once fixed in place. No heavy armour of its own; anything
serious was taken from somebody.

### Army character

Cheap, numerous, unreliable, and everywhere. Its mass is **small-workshop loitering munitions** in
quantity.

One thing the bible is explicit about and mechanics must honour: **a human-delivered charge is not a
mechanic.** Where one appears it is rare, expensive, and treated on the radio as a catastrophe. It is
never a unit type in a build menu and never a joke.

### Signature

**A force that was not there, made of things that were, arriving with the paperwork already filed.**

### Failure mode

Three, and Meridian is the faction where all three are live:

- **The gimmick.** Concealment and salvage are novel enough to carry a faction that is not actually
  good at anything, and a player who cannot win a straight fight will not play it twice. Meridian
  needs a real path to a decisive engagement, and the bible forbids helplessness anyway.
- **The chore.** If any part of its taxation requires an opponent to keep doing something, the design
  has failed. Duty is additive and the counterplay is a *choice*: fight for the node, or route around
  and pay in time.
- **The winner's bonus.** If salvage pays best to whoever is already winning, it is a snowball wearing
  a comeback mechanic's clothes. The guard is that it pays for *battles*, not for victories.
- **And the one interdiction adds: the fourth failure is over-correction.** Meridian's duty now falls
  whenever anyone fights on its network, which is a large downward pressure applied to the faction with
  the middling ceiling. If the salvage half does not rise fast enough to meet it, Meridian is simply
  poor in every game that has a war in it. The two curves have to be swept against each other rather
  than tuned separately — see [balance.md §5.4](balance.md#54-numbers-that-are-measured-not-chosen).

### Counterplay

Detect it. Pin it — it collapses once observed and cannot trade with heavy armour it did not steal.
Deny it material by fighting where its recovery vehicles cannot reach, or by winning small. And accept
the uncomfortable one the bible built on purpose: **its production is somebody's town**, and the
efficient answer to that is a decision the player has to make rather than one the game makes for them.

---

## 7. Matchup textures

Not balance — these are the *shapes* the nine matchups should have. If a matchup does not feel like its
row, something in the design has flattened.

| | **vs AEC** | **vs Concord** | **vs Meridian** |
|---|---|---|---|
| **AEC** | Two mobile forces, both fragile, decided by information | Speed and reach against mass and time; AEC must win before the wave | Vision against ambiguity — the game's cleanest opposition |
| **Concord** | Deny the sky, absorb the raids, land the wave | Two schedules; whoever guessed the map right | Concord's hardest matchup: least detection against most concealment |
| **Meridian** | Ambiguity against sweeps; every desk found is income lost | Tax a rich, slow, visible economy — and it may not be worth killing | Two invisible armies over the same nodes; a knife fight over paperwork |

Concord-versus-Meridian is deliberately Concord's worst and is *not* to be fixed by giving Concord more
detection. It is fixed, if it needs fixing, by making Meridian's economy findable by other means —
which it already is, because a desk serves a notice to the faction it charges.

**Interdiction gives every row a second, shared question: where can we afford to fight?** The answer
differs by pairing and that is the point. AEC-versus-Concord is fought deep, because AEC's carriage
ignores links and Concord's does not, so AEC wants the fighting *on* the road and Concord wants it
anywhere else. Anything-versus-Meridian inverts the usual logic — grinding on the corridor starves
Meridian's duty, so the faction being attacked economically is the one being *helped* by the attacker
choosing to fight somewhere quiet.

---

## 8. Open questions

- **Whether AEC's corridor capacity is one number or per-region.** One number is legible; per-region
  makes forward beacons matter more. Leaning toward one number until it is boring.
- **Whether Concord's batch size is fixed per factory or chosen per order.** Chosen is more expressive
  and much harder to balance against a fixed cost curve.
- **How a converted building is taken from Meridian.** Destroying it works and is unambiguous.
  *Capturing* it is more interesting and raises a question the bible would want asked — whether the
  other factions can use a converted workshop, and what it says about them if they can.
- **Whether the three factions' carriers should be visually distinguishable at combat range.** They
  must be for readability; Meridian's recovered hulls argue the opposite. Readability wins where they
  conflict, and this is the one place it is not obvious.
