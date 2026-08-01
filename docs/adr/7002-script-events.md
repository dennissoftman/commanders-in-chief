# ADR 7002: Script events — subscription is a handler, and scripts arrive with the scenario

- Status: accepted, implemented. `cic_sim::scripts` dispatches, and a scenario's scripts run.

## Context

M10 landed the language with events already in its grammar: `on <event>(args) { ... }` runs when the
engine raises that event, the event set is declared by the engine on the `Interface`, and a handler for
an event the engine did not declare is a **compile** error naming the line and listing what exists. The
compiled `Program` reports `handles(event)` and `handled_events()`. What M10 deliberately did not decide
is everything around that seam: how scripts get into a map package, how many a scenario may have, what
order they run in, which events the engine will actually raise, and how a script names a zone or a unit
when one arrives as an argument.

Denys asked for this to be planned before any of it is implemented (2026-07-29), naming the design
question directly: there are several kinds of events — level start, something moving in or out of a
zone, and many others — so either a main script subscribes to events, or it works some other way around.
This record is the other way around, and why.

Two constraints are already in force and are not this record's to revisit. Scripts run inside the
simulation, so dispatch must be deterministic to the tick and to the order — [ADR
0007](0007-simulation-arithmetic.md) and the [determinism invariants](../invariants/determinism.md).
And data may not name an action the engine does not define, failing at load rather than at trigger time —
the rule [ADR 7001](7001-scripting-language.md) built the language around.

The scenario format today has waypoints but no zones, so the events Denys named first have a format
dependency this record must sketch rather than inherit.

## Decision

1. **A handler is the subscription.** There is no registration call, no `main`, and no subscribe API: a
   script subscribes to `zone_entered` by declaring `on zone_entered(zone, unit)`. The kernel raises an
   event only to scripts whose `Program::handles` says so; a script without the handler is skipped at
   dispatch. The VM's existing rule that raising an unhandled event is an *error* stays exactly as it is —
   it guards the dispatcher against raising blind, and the `handles` check is what makes it unreachable
   in normal operation.

2. **The scenario names its scripts, and the package carries them.** `map.json` gains an optional
   `scripts` field: an ordered list of package-relative paths, e.g. `["scripts/mission.cics"]`. The
   package layer reads and compiles every listed script at load, against the engine's `Interface`, under
   caller-supplied limits like every other decoder here. A script that fails to compile fails the load,
   with the file, the line, and the diagnostic. No directory scanning: a script not named by the
   scenario does not run, however it got into the archive.

3. **Dispatch order is authored order.** When several scripts handle the same event, they run in the
   order the `scripts` array lists them, back to back, within the same tick. Determinism needs *an*
   order; authored order is the one a designer can see, diff, and change. This is the mount-order rule
   one layer up: explicit, never derived from enumeration.

4. **The event vocabulary is a closed, versioned set the kernel declares.** Proposed initial set:

   | Event | Arguments | Raised |
   |---|---|---|
   | `start` | — | Once, before the first `tick`, after spawn assignment. |
   | `tick` | `elapsed: real` | Every simulation tick; `elapsed` is the fixed tick length in seconds. |
   | `timer_elapsed` | `timer: str` | When a timer a script armed via a host verb runs out. |
   | `zone_entered` | `zone: str, unit: int` | The tick a unit's position first tests inside the zone. |
   | `zone_exited` | `zone: str, unit: int` | The tick it first tests outside again. |

   `start`, `tick`, and `timer_elapsed` are raiseable by M5's kernel alone. The zone pair needs zones
   and units to exist, so it lands with the M6 capabilities that supply them — declared here so the
   scenario format and the host verbs are designed against the real signatures. Adding an event later is
   backwards-compatible (an old script simply does not handle it); renaming or removing one is a break
   and takes a new `Interface` version.

5. **Entities cross the boundary as the identifiers the formats already use.** A zone is its scenario
   `name` (`str` — a compile-time constant in the language, so the comparison is an index test, not a
   string walk at run time). A unit is its kernel id (`int` — the stable deterministic counter the M5
   charter requires). A timer is the name the script armed it with (`str`).

6. **Zones are scenario data.** `map.json` grows a `zones` array — named, uniquely, like waypoints; each
   an axis-aligned rectangle or a circle in world units; every coordinate cross-checked against the
   terrain extent at package load exactly as positions are today. Sketched here, implemented when the
   zone events are.

7. **What a script remembers is kernel state behind host verbs.** Already the language's rule — no
   globals, nothing survives between runs — restated as the event model's consequence: mission flags,
   counters, and timers live on the kernel side of `sys.*`, where they are hashed per tick, recorded,
   and replayed with everything else. A subscription model that kept state in the script runtime would
   have created exactly the invisible-to-hash state this forbids.

8. **A runtime fault disables that script for the rest of the run.** The fault is deterministic (same
   tick, same instruction, every machine), so the disabling is too. Side effects already applied through
   host verbs stand — there is no rollback — and the fault is recorded in the run's diagnostics naming
   script, event, function, and line. The alternatives are worse in both directions: killing the match
   hands a mod author a denial-of-service on every player, and re-raising into a script that just proved
   itself wrong turns one diagnostic into one per tick.

## Rationale

**Why not a main script that subscribes at run time.** `sys.subscribe("zone_entered", ...)` needs a way
to name the handler, and the language has no function references — deliberately, since a callable value
is most of a closure. It would also move the binding to run time, which is the exact late-failure shape
ADR 7001 exists to prevent: a typo'd event name becomes a subscription that never fires, silently, on
every machine. The `on` block is already a declarative, compile-checked, diff-visible subscription; a
second mechanism would be a worse spelling of it.

**Why not bindings in `map.json`** (an event-to-function table beside the script list). It puts script
internals in a second file, so renaming a function breaks a JSON file the compiler does not read, and
validation needs a cross-check the compiler already performs for free on `on` blocks. A designer reading
the script sees every subscription; a table would mean reading two files to know what runs.

**Why multiple scripts rather than one.** The language has no modules — deliberately — so the scenario
list is the only composition mechanism: a campaign mission can carry its own script beside a faction's
shared one, and a mod can add behaviour without editing the mission's file. Scripts are isolated (no
cross-script calls, no shared namespace), which keeps "what can this mod's script do" answerable from
that script alone. One script per scenario was rejected as a limit that buys nothing: dispatch order has
to be defined even for one, and everything else scales linearly.

**Why authored dispatch order.** Any implicit order — path sort, compile order, hash order — is a
determinism hazard the moment two machines disagree about it, and an invisible one until they do. The
array is already ordered; using that order costs a sentence in the format doc.

**Why the fault policy is per-script.** Fuel already bounds a hostile script; the fault policy is about a
*wrong* one. Disabling per script is the smallest deterministic response that keeps a diagnostic visible
without letting one broken mod handler take the mission logic down with it.

## Consequences

- `map.json` gains `scripts` (this record) and later `zones` (with the zone events); both are format
  changes with the usual validation: unknown fields rejected, unique names, paths normalized, positions
  inside the terrain extent. `docs/formats/scenario.md` and `package.md` gain the sections when the
  code lands, not before.
- The kernel owns the dispatcher: it compiles the scenario's scripts at activation, keeps them in
  authored order, consults `handles` before every raise, runs each handler under the session's
  `RuntimeLimits`, and records faults. Raising an event is part of the tick, so handler execution order
  is part of the simulation's state transition — covered by the per-tick hashes M5 ships.
- The `Interface` the kernel declares becomes part of the engine's compatibility surface: its event
  names, arities, and host verbs are what a `.cicmap`'s scripts compile against, so it is versioned and
  its changes are release notes.
- `cic-script` itself needs nothing: `Program::handles`, `handled_events`, the `Interface` declaration
  path, and the fuel machinery all exist. This record is deliberately implementable without touching
  the language.
- Until M5 lands, nothing raises an event, so the `scripts` field could load and validate before it can
  run. Landing the format change together with M5's activation path keeps a green field from claiming
  more than it does — the same lesson the status file records about lines reading as done.

## What implementing it established

The dispatcher is `cic_sim::scripts`, and `map.json` carries the `scripts` array. Eight of the nine
decisions landed as written. What the implementation changed, found, or had to decide is below —
including the two places this record was wrong.

**Decision 4 was too eager, and the fix is the rule the record already contains.** The initial event
set is declared as three, not five: `start`, `tick` and `timer_elapsed`. Declaring `zone_entered` and
`zone_exited` before anything can raise them would make `on zone_entered(zone, unit)` *compile* and
then never fire — which is precisely the silent no-op decision 1 and ADR 7001 exist to prevent, and
is indistinguishable from a handler whose body is wrong. Undeclared, it is a compile error naming the
line and listing what does exist. The record's own note that adding an event later is
backwards-compatible is what makes waiting free; the zone signatures stay in the table above as the
design the scenario format and the host verbs are built against. **An event is declared when it can
be raised, not when it is designed.**

**A `str` argument cannot be synthesized, so decision 5 needed a mechanism it did not name.** A
string in this language is an index into the program's constant table and there is no heap, so the
kernel cannot manufacture one. `timer_elapsed(timer)` therefore resolves the name against *each
receiving program's own table*, and a script whose source never writes that name is not raised to.
This is the faithful reading of decision 5's parenthesis — a comparison is an index test — but the
consequence is worth stating plainly: **a script hears about a timer only if it names it.** The
alternative was handing a handler a value it could not compare against any of its own literals, which
spends fuel to accomplish nothing. It is also visible in the source rather than hidden, which is the
property the closed surface is for.

**Timers needed an ordering rule the record left open.** Three sub-decisions, all of them forced by
determinism rather than taste:

- Timers fall due **after** the tick handlers, so arming one and reacting to one are never the same
  pass.
- A timer **never fires in the tick that armed it**, even at zero seconds. Otherwise a handler
  re-arming itself loops within one tick until it exhausts its fuel — a hang expressed as a fault
  rather than as behaviour.
- Due timers are collected **before** any handler runs, and in name order. Name order is what makes
  the sequence identical on every machine; collecting first is what lets a handler re-arm its own
  name without the new timer being consumed by the pass that delivered the old one.

**Fuel is per handler, and is deliberately not hashed.** Each raise gets a fresh budget, so one
expensive handler cannot starve the next script in authored order. The peak is *reported*, for the
tuning M10 argued for, but kept out of the hash: fuel is the interpreter's accounting rather than the
mission's state, and hashing it would make every recorded replay depend on how many instructions this
crate's compiler happens to emit.

**Refusal and fault turned out to be two different responses, and the split matters.** A runtime
fault disables the script (decision 8). But a verb given a nonsensical *argument* — a negative
duration, most likely computed rather than written — only refuses, returns `false`, and increments a
counter that **is** hashed. Taking a script out of the run for arithmetic that came out wrong is
heavier than the mistake deserves; leaving the refusal invisible is what the counter prevents. This is
`Units::rejected`'s lesson applied one layer up: a machine that refused a different number of verbs
diverges on the tick it happened. A counter that would *overflow* is the other way round — a fault,
because a counter that silently stops counting is a mission rule that silently stops firing.

**Decision 7 landed as the thing that makes the rest cheap.** Because mission memory is kernel state,
the dispatcher needed no new hashing machinery, no snapshot path, and nothing at all in the replay
format: flags, counters and timers fold into the existing per-subsystem hash, and a script that
behaves differently on two machines diverges on the tick it happened, attributed to `scripts`. A test
plants exactly that and reads the tick back.

**`cic-sim` now depends on `cic-script`**, which is a new edge in the dependency graph and the
direction the ADR implies: the kernel owns the dispatcher, so the kernel takes the language. Nothing
reverses — `cic-script` still depends on `cic-math` and nothing else, which is what keeps the sandbox
a property of the graph rather than of what the VM happens not to call.

**What is still outstanding**, and why it is not here: the host verbs that reach *other* subsystems —
spawn, order, count, query a zone. The mission verbs in this dispatcher touch only mission state, so
they need nothing but `&mut self`. A verb that spawns a unit has to reach the `units` subsystem from
inside another subsystem's tick, which is a kernel question about cross-subsystem access during a
tick rather than a scripting one, and it is deliberately not answered by pretending a script may
forge a player's command.
