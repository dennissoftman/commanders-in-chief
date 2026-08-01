# Balance

**Status:** working draft. The framework is proposed for use *before* the numbers exist, which is the
point of it.
**Scope:** how a number gets chosen, what it is measured against, and which numbers are allowed to be
opinions. Reference values are here; they are provisional and labelled as such.

M6 states plainly that it does no balance — "numbers exist to make mechanics testable, not to be
fair" — and that is the right position for that milestone. This document exists so that when balance
*does* start, it starts with a method rather than with a spreadsheet somebody nudged.

---

## 1. The three kinds of number

Every quantity in the game is one of these, and knowing which one it is settles most arguments about it.

| Kind | How it is set | Who may change it | Example |
|---|---|---|---|
| **Anchor** | Chosen once, arbitrarily, to define a scale | Nobody, without re-deriving everything below | Load value = 100 credits |
| **Derived** | Computed from anchors by a published formula | The formula, not a person | A unit's cost |
| **Measured** | Swept by the harness until a stated target is hit | The harness | Meridian's duty rate |

**A derived number that has been hand-edited is a defect** unless the departure is annotated with a
reason. That rule is enforced by a test ([§7.1](#71-the-budget-line-test)), and it is the single most
important mechanism here: a roster of two hundred units drifts into incoherence one justified
exception at a time, and the only defence is making each exception say so out loud.

---

## 2. Arithmetic

**Everything is an integer.** Costs, healths, damages, ranges, rates, income. The kernel is almost
entirely integer by construction ([ADR 0007](../adr/0007-simulation-arithmetic.md)) and a balance
model that accumulates fractions is a determinism liability bought for precision nothing needs.

Where a rate does not divide evenly into a tick, it accumulates as an **integer numerator over a fixed
denominator with the remainder retained** — so income of 1200 per minute at 30 Hz is 1200 per 1800
ticks, added as `+1200` to a numerator each tick and yielding `numerator / 1800` credits with the
remainder carried. Exact, driftless, hashable, and identical on every machine.

Damage multipliers are integer percentages with integer division. `40 × 150 / 100 = 60`, and
`40 × 125 / 100 = 50`. Truncation is the specified behaviour, not a rounding error to be corrected.

**Time is quoted in ticks and seconds both.** The tick rate is 30 Hz, so a second is 30 ticks. Design
conversation happens in seconds; the data holds ticks.

---

## 3. The budget line

### 3.1 Durability

An armour class's **durability index** is `100 ÷ (the average multiplier that lands on it)`, taken
across the four damage types in [mechanics.md §3.2](mechanics.md#32-damage-types-and-armour-classes):

| Armour class | Average incoming | Durability index |
|---|---|---|
| infantry | 125 | 0.80 |
| light | 112.5 | 0.89 |
| structure | 87.5 | 1.14 |
| heavy | 81.25 | 1.23 |
| air | 81.25 | 1.23 |

`EHP = health × durability index`. A point of infantry health is worth less than a point of tank
health, which is correct and is what stops infantry rosters from being priced as cheap tanks.

### 3.2 Combat value and cost

```
DPS   = damage × 30 ÷ cooldown_ticks
EHP   = health × durability_index
value = sqrt(DPS × EHP)
cost  = 2.5 × value × Σ(modifiers)
```

The square root is doing real work and its consequence should be understood before anyone objects to
it: it makes **massed cheap units cost-efficient**, because value grows with the square root of
concentrated firepower while cost grows linearly. That is the correct default for this genre — it is
why a horde is a viable answer to a heavy — and it means a heavy unit's premium has to be justified by
something the formula does not capture: range, role coverage, mobility, survivability against a
specific threat. Those are the modifiers, and they are the honest place for that premium to sit.

### 3.3 Modifiers

Multiplicative, applied to cost, all provisional:

| Modifier | Value | Why |
|---|---|---|
| Range above reference (80) | +2% per 10 units | Range is the strongest untaxed advantage in an RTS |
| Speed above reference | +3% per 10% | |
| Vision above reference (140) | +1.5% per 10 units | |
| Cannot engage ground, or cannot engage air | ×0.6 | A dedicated role is worth less than a general one |
| Concealment | ×1.25 | |
| Transport capacity | flat, per slot | Not a combat property; do not launder it through `value` |
| Recovered (Meridian), unreliable | ×0.90 | On top of the reduced health, which the formula already sees |

### 3.4 Worked examples

Provisional, and shown because a formula without worked examples is not checkable:

| Unit | Health | Armour | EHP | Damage | Cooldown | DPS | value | Modifiers | Cost |
|---|---|---|---|---|---|---|---|---|---|
| **Line vehicle** *(the reference)* | 400 | light | 356 | 40 kinetic | 30 | 40 | 119 | — | **300** |
| Infantry squad | 300 | infantry | 240 | 24 kinetic | 30 | 24 | 76 | — | 190 |
| Heavy tank | 1200 | heavy | 1476 | 90 heat | 60 | 45 | 258 | +8% range | 700 |
| Anti-air | 350 | light | 311 | 50 frag | 30 | 50 | 125 | ×0.6 air-only | 190 |
| Artillery | 300 | light | 267 | 150 blast | 150 | 30 | 89 | +64% range, ×0.6 no air | 220 |

The line vehicle is the **anchor**: its cost, health and damage are chosen, and the coefficient 2.5
exists to make the formula reproduce them. Everything else in the table is derived.

### 3.5 Time to kill

Targets, because pacing has to be a decision rather than an emergent accident:

| Engagement | Target TTK | Why |
|---|---|---|
| Mirror, line vehicle | 6–8 s | Long enough that withdrawing is a real option |
| Correct counter | 2.5–4 s | A right guess should feel decisive |
| Wrong counter | 15–25 s | A bad trade, never a null one |
| Anything vs a structure | ≥ 20 s | A base should not evaporate off screen |

The mirror figure falls out of the anchors: `400 health ÷ (40 × 150 / 100) = 6.7 s`. That it lands
inside the target range without tuning is the check that the anchors are consistent, not a coincidence.

---

## 4. What the factions are allowed to differ by

### 4.1 The rule

**Doctrine lives in the economy and the production layer. Statlines are where balance is achieved.**

A faction that needs a 40% better tank to feel distinct has been designed wrong, because it has spent
its identity budget in the one place that makes cross-faction comparison impossible. AEC feels
different because its force arrives from off-map at a pad it chose; Concord because its wave was
committed four minutes ago; Meridian because its army is made of the war. None of that is a statline.

### 4.2 The asymmetry budget

A faction's fill of a tactical role may sit within **±20% of the budget line** in cost efficiency.
Anything beyond that must be paid for in a **named dimension**, recorded on the template, drawn from
this closed list:

- mobility, sight, range
- ramp time — how long before it is available
- ceiling pressure — how much of the faction's structure-count limit it consumes
- delivery constraint — where it can arrive
- reliability

The list is closed on purpose. An open one becomes "it is worse in a way nobody measured".

---

## 5. Economic benchmarks

### 5.1 The reference map

Balance is stated against one authored map, and every figure below is meaningless without it:

| Property | Reference 1v1 value |
|---|---|
| Gates | 2 |
| Gate rate | one 100-credit load per 150 ticks (5 s) |
| Total map flow | 2400 credits/min |
| Yards | 6 — two home, four contested |
| Inflow per yard | 400 credits/min — total flow over yard count |
| Route nodes | 8 |
| Bridges | 3, each with exactly one alternate path |
| Detour penalty per severed crossing | +40% round-trip time on the affected link |
| Convertible buildings | 12, in two clusters |
| Carrier capacity | 100 — one load |
| Carrier round trip, reference distance | 240 ticks (8 s), so 750 credits/min |
| Starting credit | 3000 |
| Starting carriers | 2 |

### 5.2 Targets

| Benchmark | Target |
|---|---|
| Income, per worked yard | 400 credits/min — so income ≈ `400 × yards worked` |
| Parity income, per player | 1200 credits/min — three yards, four line vehicles a minute |
| Income at 4:00, uncontested opening | 800 credits/min — two yards |
| Income at 10:00, one expansion held | 1600 credits/min — four yards |
| First combat unit | T+30 s |
| First contact | T+90 s |
| First committed engagement | T+3:00 |
| Expansion break-even | 100 s of operation |
| Decided match | 15–25 min |
| Duty as a share of Meridian's parity income | ≤ 40% |

### 5.3 The property that matters most

**Income is flow-limited, not carrier-limited.** A single carrier moves 750 credits/min at reference
distance and a yard only supplies 400, so **one carrier saturates a yard** and a second on the same
yard earns nothing. That is deliberate, and it is the mechanism behind [mechanics.md
§2.7](mechanics.md#27-why-expansion-is-a-real-decision): the way to earn more is to hold more of the
map, never to buy more trucks. A model where stacking carriers is straightforwardly good produces an
opening with one correct answer and no decisions in it.

Two consequences fall out, and both are better decisions than the one they replace:

- **A carrier's saturation point is a distance, not a count.** A yard at twice the reference distance
  needs two carriers to clear its inflow, so the purchase is tied to *how far away the income is* —
  which is "distance is money" arriving a second time, from the other side.
- **Beyond saturation, carrier count is insurance.** A spare covers an interception without a gap in
  income. The two starting carriers against one home yard are therefore deliberately
  over-provisioned: the second is idle until the player takes a second yard, which is the opening's
  first real decision and is visible in the numbers rather than in a tutorial.

### 5.4 Numbers that are measured, not chosen

These are set by sweeping until the stated target is met, and nobody may hand-pick them:

| Quantity | Target it is swept against |
|---|---|
| Meridian's duty rate | Duty ≤ 40% of Meridian's parity income on the reference map |
| AEC's corridor capacity per beacon | AEC parity income within 5% of 1200/min at equal map share |
| AEC's sortie replacement cost and slot downtime | A downed sortie costing 1.5–2.5× what an intercepting unit risked |
| Contractor fee per load | Contracted income at 50–65% of the air bridge's, at equal distance |
| Concord's batch size and lead time | Concord reaching parity income by 8:00 and exceeding it after 12:00 |
| Concord's grading cost and re-grading discount | A crater-and-repave exchange costing the attacker less credit and the defender more time |
| Meridian's porter throughput | Meridian's own collection ≥ 60% of parity without duty |
| Salvage recovery cost and wreck decay | Salvage supplying ≤ 30% of Meridian's fielded value at 15:00 |
| **Interdiction radius and hold duration** | A skirmish that both sides walk away from closing a link for under 30 s; a running battle keeping it shut |
| **Upstream backlog cap** | A link shut for 60 s returning ≤ 50% of what it would have carried, as a surge |
| **Link capacity per road condition** | A metalled link carrying ≥ 2× a cratered one, and a plain link never below the gate rate feeding it |
| **Cratering: damage per munition, and repair cost and time** | An artillery mission costing the attacker less credit than the repair costs the defender, and less time |
| **Wreck cost class and decay** | A link needing the wreckage of ~a company before it closes, and clearing itself within ~90 s of the last loss |
| **Bridge health, demolition cost, and rebuild cost and time** | Dropping a crossing costing meaningfully more than cratering, and a rebuild that an opponent overwatching the site can contest but not trivially deny |
| **Duty against salvage, swept together** | Meridian's total income varying by ≤ 25% between a quiet map and a bloody one |

The eighth row is the anti-snowball guard from
[faction-mechanics.md §6](faction-mechanics.md#6-meridian--meridian-council) expressed as a number
something can check.

**The last row is the one that cannot be swept one variable at a time.** Interdiction pushes Meridian's
duty *down* as fighting rises and salvage pulls its income *up* on the same axis, so tuning either
alone produces a faction that is poor in every war or rich in every one. They are one target with two
knobs. It is also the only place in this document where a faction's two income sources are deliberately
anti-correlated, and the reason is character rather than balance — see
[the Council and the Chain](faction-mechanics.md#the-council-and-the-chain-sit-on-opposite-sides-of-the-war).

The backlog cap deserves its own note: a link that reopens must **not** return everything it would have
carried, or interdiction costs the victim nothing in the long run and the whole mechanic is theatre. It
must also return *something*, or breaking a blockade pays nothing and nobody breaks one. Half is the
starting guess and it is a guess.

---

## 6. Verification

This is the half that makes the rest more than a document. The project's standing position is that a
green suite is not verification when the fixture can be the bug — so the checks below are split
deliberately into **invariants that fail a build** and **statistics that are tracked and looked at**.

### 6.1 Invariants — these fail the build

| Test | Asserts |
|---|---|
| **The budget line** | Every unit's cost is within ±10% of the formula, or carries an annotated exception naming a dimension from the closed list |
| **No immunity** | No entry in the damage table is zero |
| **Credit conservation** | Credits created equal gate output collected plus duty assessed, less contractor fees. Nothing else creates credit, and the total is part of the tick hash |
| **Interdiction is symmetric** | Damage from either side holds a link shut. A test plants a one-sided engagement and asserts the link stays closed while the *defender* is the only one still shooting |
| **Interdiction needs an owner** | Damage to neutral scenery, props, wrecks and unclaimed buildings closes nothing. This is the rule most likely to be broken by a later refactor that makes everything damageable uniformly |
| **Combat does not degrade the road** | A battle fought to completion on a stretch of road leaves that stretch's cost class unchanged. Only ordnance aimed at the road moves it |
| **A cratered road still carries** | A link at the worst reachable road condition, with no fighting and no wreckage, has capacity above zero. Only a demolished structure or an obstructed roadway reaches zero |
| **Severance reroutes** | With a bridge destroyed and an alternate path present, flow continues on the longer path and does *not* accumulate upstream. Upstream accumulation happens only when the graph offers no path at all |
| **Severance does not interdict** | Demolishing a bridge with no owned asset nearby leaves every link's interdiction state clear. A bridge is neutral, so destroying it severs without triggering the fighting rule |
| **Reachability** | Every template is buildable from a stock start on the reference map — no orphan tech |
| **Economic benchmark** | Income at one, two and three worked yards is within tolerance of §5.2 |
| **Role coverage** | Every faction has at least one template in every tactical role |

Credit conservation is the one worth arguing for: an income bug that quietly doubles a rate is
invisible in play until somebody notices a match is fast, and it is *exactly* the kind of defect this
project's per-subsystem hashing was built to catch on the tick it happens.

### 6.2 Statistics — these are tracked, not enforced

| Run | Reports |
|---|---|
| **Mirror sweep** | Each faction against itself, identical AI, symmetric map, *N* seeds. Neither seat should exceed 55% |
| **Matchup sweep** | All nine matchups, *N* seeds, headless. Win rate and match duration per cell |
| **Opening clock** | Time to first unit, first contact, first engagement, first expansion |

A 53% win rate must not fail CI. Balance is a moving target and a red build for noise trains people to
ignore red builds; these run headless, publish as an artifact, and are read.

### 6.3 The caveat that makes the sweeps honest

**A matchup sweep measures the AI as much as the balance.** M6 scopes the AI as a test harness — good
enough to exercise every mechanic unattended — and such an opponent will not play three asymmetric
factions equally well. It will be better at Concord, whose plan is a schedule, than at Meridian, whose
plan is concealment and opportunism.

So the sweeps are a **signal for investigation and never a verdict.** The load-bearing checks are the
mirror sweep, which cancels AI skill by construction, and the budget line, which does not involve the
AI at all. This is the project's own standing warning applied to a new domain: a fixture that cannot
show what it measures is indistinguishable from one that passes, and a balance harness driven by an AI
that plays one faction badly will report that faction as weak forever.

Determinism is what makes any of this affordable. A seeded match is reproducible to the tick, so a
sweep can be re-run against a changed number and the difference attributed.

---

## 7. Process

### 7.1 The budget-line test

The mechanism, stated concretely because it is the one thing here that has to exist in code: the
template set carries each unit's statline and its cost, a test recomputes the cost from the formula,
and a mismatch beyond ±10% fails unless the template carries an exception field naming a dimension and
a reason. Changing a cost without recording why is therefore a build failure, and the annotation shows
up in a diff where a reviewer sees it.

### 7.2 When numbers change

- **A change to an anchor re-derives the roster.** Anchors are not tuning knobs; touching one is a
  deliberate act with a commit message explaining it.
- **A change to a derived number is a change to the formula or an annotated exception.** There is no
  third option.
- **A change to a measured number is a sweep result**, with the target it was swept against cited.

### 7.3 Where the numbers live

In `templates.json` ([the format](../formats/templates.md)), which is the data the game reads —
never in this document. This file holds the anchors, the formula, the targets and the method. A
document holding live values is a document that goes stale silently, and the format already rejects
unknown fields precisely so a typo is a loud error rather than a balance bug.

---

## 8. Explicitly not balanced yet

- **Nothing.** No number in the tree has been through this framework, because the mechanics that read
  them do not exist yet. That is the correct order: M6's charter says numbers exist to make mechanics
  testable, and this document is what the *next* pass uses.
- **The reference map does not exist.** Section 5 describes a map that has to be authored, and every
  figure in this document is conditional on it.
- **Team and three-way play.** Every figure here is one-versus-one. A three-way match changes the
  economy qualitatively rather than quantitatively — Meridian taxes two opponents, and two players can
  agree to ignore a third — and it needs its own targets.
- **The AI's own balance.** Difficulty levels are a separate subject from faction balance and must not
  be tuned by adjusting faction numbers.
