# ADR 3005: The route graph — sites are vertices, links are drawn, flow is routed like paths

- Status: proposed

Numbered in the `3xxx` family — simulation — for the same reason [ADR 3002](3002-corridor-economy.md)
was: the graph is kernel state, its flow is hashed, and two machines holding different graphs are
playing different games.

## Context

[ADR 3002](3002-corridor-economy.md) builds the whole corridor economy on "an authored route graph"
and never decides what one is. Its decision 16 places the graph as the middle of three layers —
authored topology over the corridor, with carriage deliberately *not* bound to it — and then decisions
8, 15, 17 and 21 all lean on structure nobody has recorded: gates feed the graph, interdiction acts
"within the link's radius", freight is "a rate along the graph", and severance takes "the next-best
path". Its Consequences promise that "`map.json` gains three optional members" without naming them.
Issue #76 item 3 names the gap outright: no scenario fields, no spatial definition of a link, and it is
genuinely undecided whether gates and yards are graph vertices — which sets Meridian's income ceiling.

The design documents have a seam in exactly that place. [mechanics.md
§2.2](../design/mechanics.md#22-the-three-layers-of-a-supply-chain) says the graph is "gates, yards,
nodes, and the links between them"; [§2.3](../design/mechanics.md#23-the-maps-economic-furniture) says
nodes are "the vertices of the route graph". Both cannot be right, and the difference is not
pedantry: [§7](../design/mechanics.md#7-what-a-map-author-controls)'s third knob prices Meridian's
income ceiling by "route node count", so what counts as a node is a balance number.

Constraints already in force: [ADR 0007](0007-simulation-arithmetic.md)'s arithmetic and the
[determinism invariants](../invariants/determinism.md); [ADR 3001](3001-pathfinding.md)'s ground grid,
whose Consequences this record must not duplicate — the corridor is *cells*, the graph is topology
over them; the scenario format's own rules — additive optional members, unknown fields rejected,
cross-references checked by the package loader; and [balance.md §5](../design/balance.md#5-economic-benchmarks),
whose reference map (gates 2, yards 6, route nodes 8, bridges 3 each with exactly one alternate path)
has to be expressible in whatever is decided here.

## Decision

1. **Gates, yards and nodes are all vertices of one graph.** A graph whose sources and sinks were not
   on it could not route flow from a gate to a yard without a second, unwritten rule attaching them.
   **"Route nodes" are the interior vertices** — the only kind Meridian may build a permit desk at —
   so §7's knob and Meridian's income ceiling stay priced by node count exactly as written, and a map
   author cannot inflate that ceiling by adding gates or yards. mechanics.md §2.3's sentence tightens
   to "the *interior* vertices of the route graph" on acceptance; that is a follow-up edit, not part
   of this record.

2. **`map.json` gains `gates`, `yards` and `routes` — the three optional members ADR 3002 promised.**

   ```json
   "gates": [
     { "id": "west-gate", "position": { "x": 0.0, "y": 512.0 }, "load": 100, "period": 150 }
   ],
   "yards": [
     { "id": "mill-yard", "position": { "x": 300.0, "y": 480.0 }, "capacity": 6 }
   ],
   "routes": {
     "nodes": [ { "id": "junction-a", "position": { "x": 160.0, "y": 500.0 } } ],
     "links": [
       { "from": "west-gate", "to": "junction-a" },
       { "from": "junction-a", "to": "mill-yard", "via": [ { "x": 220.0, "y": 470.0 } ] }
     ]
   }
   ```

   A gate is a map-edge site with a rate: a `load` value in credits and a `period` in ticks —
   [balance.md §5.1](../design/balance.md#51-the-reference-map)'s "one 100-credit load per 150 ticks"
   is those two numbers verbatim. A yard is a positioned site with a `capacity` in loads, per ADR 3002
   decision 1's "a full yard stops accepting". A node is a positioned interior vertex. A link is a
   pair of vertex references with an optional `via` polyline (decision 5). All three members are
   optional and default empty — a map without them has no economy and stays valid — and unknown
   fields are rejected, per the scenario format's standing rule.

3. **Identity is authored order.** `id` strings share one namespace across gates, yards and nodes,
   because a link may reference any of them. At activation, vertices take indices in authored order —
   gates, then yards, then nodes, each in array order — and links likewise, exactly as activation
   already seats players and numbers placements. Wherever this record says a tie breaks, it breaks on
   the lower index, and nothing else about the order decides anything.

4. **Every cross-check lives in the package loader**, which already validates what neither format can
   alone. The checks: every `id` unique; every link endpoint resolves to a declared vertex; every
   position inside the terrain's world extent; every gate within one cell interval of the extent's
   edge (positions are authored floats, so "exactly on the boundary" is a rule authors would fight);
   no self-loops; no duplicate links, compared as unordered pairs; **every gate reaches at least one
   yard**, and **every vertex lies on at least one link** — both load errors, argued below. What the
   loader deliberately does *not* check is traversability of the ground under a link: passability is
   the simulation's derivation and stamps land on tick zero, so a link drawn over water is valid at
   load and simply has capacity zero until somebody's bridge grants passage.

5. **A link's geometry is authored: its two endpoints, plus the optional `via` waypoints, joined by
   straight segments.** It is drawn by the map author along the road the author is drawing anyway,
   and it is **never derived by pathing over the ground grid** — not at load, not at runtime. Links
   are undirected: freight routes over them in whichever direction its path needs, no mechanic reads
   a direction, and carriage is not bound to them at all (ADR 3002 decision 16).

6. **A link's capacity is the dearest cost class under its geometry, through a table.** The cells a
   link's segments pass through are enumerated by the same deterministic line walk that already
   prices a string-pulled shortcut (ADR 3001 amendment E) — one cell of width, no corridor — and the
   worst rung crossed sets the capacity, by a class→capacity table that
   [balance.md §5.4](../design/balance.md#54-numbers-that-are-measured-not-chosen) sweeps as its
   "link capacity per road condition" row. Two constraints on the table rather than on the code:
   every class cratering can reach maps *above* zero, because a cratered road still carries
   ([§6.1](../design/balance.md#61-invariants--these-fail-the-build)'s invariant); `IMPASSABLE` maps
   to zero. **Severance therefore needs no mechanism of its own**: demolishing a bridge lifts its
   `passage`, the cells under the link revert to what the derivation says — water — the walk crosses
   an impassable cell, and capacity reaches zero through the same lookup as everything else.
   Condition and obstruction arrive the same way, because grading and wrecks both edit cells (ADR
   3001 decision 3 and amendment C). One mechanism, three of ADR 3002's causes.

7. **A link is a pipe: a per-tick ceiling and a transit delay, both derived from the same walk.** The
   ceiling is decision 6's capacity. The delay is the walk priced at the search's own `10`/`14`
   against each entered cell's class, divided into ticks by a pace coefficient — so a cratered link
   carries less *and* carries slower, and a detour is longer in exactly the units the grid already
   measures. The coefficients live in a `RouteRules` gathered like `GroundRules`, for the same
   reason: a machine with a different capacity table is playing a different game.

8. **Freight is an integer rate, routed over shortest paths.** A gate emits `load` per `period` as a
   numerator over a fixed denominator with the remainder retained — [ADR
   0007](0007-simulation-arithmetic.md) as the kernel and [balance.md
   §2](../design/balance.md#2-arithmetic) already practise it. A gate's rate divides equally among
   the yards it can currently reach that are not full, remainder retained per share, which is what
   makes §5.1's "inflow per yard = total flow over yard count" arithmetic rather than aspiration.
   Each share flows along the shortest path by summed link travel cost, ties broken on the lower
   vertex and link index — ADR 3001 decision 5's determinism applied to a second, much smaller graph.
   **The load object materialises at the yard**, when the yard's accumulated numerator crosses the
   load value: ADR 3002 decision 17's traffic stays presentation, and decision 1's atom is born where
   carriage picks it up.

9. **Routing recomputes on the tick the graph changes, and only then.** The changes: a link's
   capacity reaching zero or leaving it, a yard filling or draining below full, and the underlying
   cells changing — `Ground` already reports the rectangles it edited each tick, so a link whose
   walk crosses a reported rectangle re-samples that tick, the same replan-on-change shape as ADR
   3001 decision 7. When a path dies with no alternative, flow accumulates as backlog **at the vertex
   feeding the dead link**, against ADR 3002 decision 15's cap; flow above the cap is lost, because a
   blockade that costs nothing in the long run is theatre; when the link reopens the backlog drains
   as a surge at the link's own capacity.

10. **The graph is a `cic-sim` subsystem, registered after `Ground`, and hashed whole.** It reads
    `Ground` as an earlier peer for cell classes, exactly as `Ground` reads `Forces`. The authored
    topology and `RouteRules` fold into the subsystem hash at activation, like the grid fingerprint —
    two machines with different graphs diverge on tick zero, attributed. Live state — per-link
    capacity and hold, per-vertex backlog numerators, per-yard stock — hashes per tick, and unlike
    the grid it is hashed in full rather than fingerprinted: a reference map holds tens of vertices,
    not tens of millions of cells, so ADR 3001 decision 8's economy buys nothing here.

## Rationale

**Why all three site kinds are vertices.** The alternative reading of mechanics.md §2.3 — a graph of
nodes only, with gates and yards attached by proximity to their nearest node — was considered and is
the trap this record exists to close. Proximity attachment is a hidden rule: adding a node *moves*
which vertex a yard feeds from, silently, and the attachment distance becomes an undocumented constant
that is really a graph edit nobody authored. Putting sources and sinks on the graph makes every route
an explicit authored fact, and reserving the word "node" for the interior keeps the one balance number
that hangs on the word — Meridian's ceiling — meaning what §7 already says it means.

**Why a link is drawn rather than derived.** Deriving a link's path with the ground pathfinder between
its endpoints is the seductive alternative: it tracks the real road for free, never disagrees with the
terrain, and costs the author nothing. It was rejected for three reasons, each sufficient alone.
First, interdiction needs a stable shape — ADR 3002 decision 15's "within the link's radius" must not
move because a road was graded, and a derived path re-routes whenever the grid changes *for unrelated
reasons*: a depot stamped beside the road shifts the cheapest path sideways, and with it the geometry
that decides whether a fight closes the link. Second, the detour must be knowable in advance —
mechanics.md §2.6's shaping play is built on "the graph's next-best path, so it is knowable in
advance", and a graph whose links silently re-route is one a player cannot read. Third, it couples
the hashes: the graph would become a function of `GroundRules` and every stamp, so a pathfinding
change would move economy state, and a desync in one subsystem would surface attributed to the other.
Against all that, the cost of drawing is small and already paid — the author drew the corridor's
cheap cells and will draw its texture; the polyline is the same gesture a third time, and tooling can
eventually make it one gesture.

**Why the authored line, not "the cheapest route along it".** mechanics.md §2.6 says a link's
capacity "derives from the cheapest route along it", which read literally is a constrained search in
a corridor around the link — the derived-geometry problem readmitted in miniature, with capacity
depending on verge cells the author never drew and changing when unrelated stamps land beside the
road. This record reads the authored polyline *as* the route: freight is abstract, the traffic that
would weave around a wreck is presentation, and the canonical route is the one the author drew. On
acceptance §2.6's phrase tightens to "the route the author drew", the same follow-up edit as §2.3's.

**Why the bottleneck, not an average.** A chain's capacity is its narrowest point. A link that is
metalled for a kilometre and cratered for ten metres carries what the crater admits, and an average
would let an author or a repair crew buy capacity back by improving cells that were never the
problem. The dearest rung crossed is also the only definition under which one demolished structure —
one impassable cell — reaches zero without a special case.

**Why an unreachable gate is a load error rather than a published fact.** The same reasoning that put
the extent check in the package loader: a gate that reaches no yard emits into nowhere, so the map
silently has less economy than its metadata declares — which is worse than a loud failure, because
ADR 3002 decision 14 makes that metadata a promise players choose maps by. A linkless vertex is the
same defect smaller: a node on no link pads Meridian's published ceiling with a desk site no load can
ever cross. Both are authoring debris, and the format's whole philosophy is that authoring errors
fail the load with a message rather than surface in play.

**Why undirected links.** A direction would be one more authored fact per link, consumed by nothing:
freight already gets a direction from its route, duty is assessed on crossing regardless of heading,
and carriage ignores the graph entirely. Directed links become worth revisiting if a mechanic ever
wants one-way flow, and that mechanic's record can add the field additively.

### Rejected

- **Deriving link geometry with the ground pathfinder** — rejected above; the load-bearing rejection
  in this record.
- **A constrained search for the "cheapest route" under a link** — rejected as the same problem in
  miniature; the authored polyline is the route.
- **Averaging cell classes into capacity** — rejected: a chain's capacity is its narrowest point.
- **Gates and yards attached to a nodes-only graph by proximity** — rejected: a hidden rule that
  re-routes flow when a node is added.
- **A directed graph** — rejected as authored state nothing consumes.
- **Deriving the graph from the road paint** — rejected: ADR 3002 decision 16 makes the graph
  *authored* topology, and a graph inferred from texture layers changes when an artist retouches a
  verge.
- **Unreachable gates as published metadata** — rejected: a map that quietly under-delivers its own
  declared flow is the failure the metadata exists to prevent.
- **Recomputing routes every tick** — rejected: the graph changes rarely and ADR 3001 decision 7
  already established replan-on-change; a per-tick recompute buys identical answers at a standing
  price.

## Consequences

- [scenario.md](../formats/scenario.md) gains the three members and their validation rows, and
  [package.md](../formats/package.md)'s cross-checks section grows decision 4's list — format
  documents change when the loader does, per their own convention.
- Two design-document sentences tighten on acceptance, as follow-up edits: mechanics.md §2.3's "the
  vertices of the route graph" becomes "the interior vertices", and §2.6's "cheapest route along it"
  becomes "the route the author drew".
- ADR 3002 decision 14's published figures — gate count and rate, yard count, route node count —
  become computable at load from these members, rather than being a second authored list that can
  disagree with the first.
- [balance.md §5.1](../design/balance.md#51-the-reference-map)'s reference map becomes expressible,
  which unblocks three of §6.1's build-failing invariants — severance reroutes, severance does not
  interdict, a cratered road still carries — all of which are assertions about this subsystem.
- **The author draws the road twice**, as paint and as polyline, and nothing forces them to agree. A
  polyline drawn off the road samples the verge's dearer cells and the link under-carries — visible
  in play, attributable with an editor overlay, and honestly the price of stable geometry. The
  overlay is M8's to schedule, not this record's.
- A second shortest-path implementation enters the kernel and must be kept deterministic. It is
  dozens of vertices where the grid is millions of cells, and it follows ADR 3001's shape exactly,
  but it is still a second one.
- Interdiction (ADR 3002 decision 15) gets the stable geometry its radius needs, and nothing else:
  the hold timer is per-link state this subsystem carries, but the damage events that start it are
  combat's to send, so the wiring waits on combat's record.

## What this record does not decide

- **Interdiction's radius and hold duration** — measured, not chosen; balance.md §5.4's row.
- **The damage events that trigger interdiction** — combat's record owns what "damage to something
  owned" is and how it reaches this subsystem.
- **Bridges as structures** — their health, demolition cost, and rebuild belong to construction and
  combat; to this record a bridge is only a `passage` whose lifting drives decision 6.
- **How accumulated wreckage compounds into dearer classes** — the wreck record's, adjacent to ADR
  3001 amendment C's own open question; this record only promises the capacity table has rungs for
  whatever it stamps.
- **The load object and carrier behaviour** — ADR 3002 decisions 1 and 9, implementation work; this
  record decides only where the load is born.
- **Meridian's permit desk mechanics** — only *where* one may stand is decided here: an interior
  vertex.
- **Air sorties** — ADR 3002 decision 19's sorties traverse no links, and this record inherits that
  as a boundary: nothing here applies to AEC's carriage leg.
- **The graph's query surface for the AI** — the AI record decides what it reads; this record only
  guarantees the answers are deterministic.
