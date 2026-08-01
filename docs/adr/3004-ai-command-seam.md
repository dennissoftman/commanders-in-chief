# ADR 3004: The AI command seam — the opponent is a player, not a subsystem

- Status: proposed

## Context

[M6](../milestones/m6-gameplay.md) charters *an AI opponent good enough to be a test harness — it
exercises every mechanic without a human*, and its exit condition is a full skirmish against that AI
with **no desync between a live run and its replay**. The design notes scope the opponent
deliberately: being able to run a full match unattended, repeatably, is worth more than being
challenging. [balance.md §6.3](../design/balance.md#63-the-caveat-that-makes-the-sweeps-honest)
then leans on it — the matchup sweeps are seeded headless matches driven by this AI, and they are
honest only if a seeded match reproduces.

What has not been decided is where the AI *runs* relative to the deterministic kernel, and the
choice is unretrofittable. Inside the kernel, the AI is a subsystem: named random streams, peer
reads, per-subsystem hashing, `first_divergence` attribution — inside the determinism claim, unable
to desync because nothing inside the claim can. Outside the kernel, the AI is a host-side actor
that reads simulation state and emits commands — and an actor that does that *non-deterministically*
silently desyncs every replay, because the replay re-derives what the live run decided and derives
something else. Everything M7 records and everything the sweeps compare hangs on getting this seam
right before the AI exists, for the same reason [ADR 0007](0007-simulation-arithmetic.md) was
settled before the gameplay maths: discovering the constraint after the code exists means rewriting
the code.

The seams the kernel actually offers are already built and already in use. Commands are tick-stamped
opaque payloads from a seat (`Command { tick, player, payload }`), recorded in a `CommandLog` that
refuses reordering, folded into each tick's hash so differing *inputs* are caught on the tick they
differ. Snapshots are `Kernel::subsystem` — `&dyn Subsystem`, downcast for reading, mutation
structurally impossible without the `&mut` only the tick path holds. And the terrain viewer's
standing orders are a working miniature of the whole question: `demo_orders` reads the units
snapshot between ticks and emits move commands that CURRENT.md describes as *host-side inputs of
exactly the shape a network session would feed*. [ADR 7002](7002-script-events.md) closed the other
door from the other side — its outstanding host-verbs question is deliberately not answered *by
pretending a script may forge a player's command*, and the AI raises the identical channel question
one layer out.

## Decision

1. **The AI is a host-side actor, outside the kernel and outside its determinism claim.** It
   occupies a seat — a `PlayerId` assigned by the session like any other — and everything it does,
   it does through the interfaces a human's interface uses: it reads snapshots and it emits
   commands. There is no AI subsystem, no kernel-side hook, and no command channel that bypasses
   the log.

2. **It reads only the post-tick snapshot.** After `Kernel::advance` returns, the host hands the AI
   `&Kernel`; it reads subsystems by name and downcast, exactly as presentation does. It never
   holds a `TickContext`, never sees `Peers`, never observes mid-tick state — the borrow rules make
   the mutation impossible and this decision makes the *timing* part of the contract rather than an
   accident of the borrow. Until fog of war lands, the snapshot is the whole world and the AI is
   omniscient; when fog lands, the AI reads its seat's visibility view like the interface does, and
   an AI that peeks past it is cheating in the ordinary game-design sense, not a determinism fault.

3. **It runs between ticks, on the host's schedule, and never acts on the tick it observed.** The
   host advances tick *N*, lets the AI read the end-of-*N* snapshot, and stamps whatever it decides
   for tick *N+1* at the earliest — under M7's lockstep, for whatever the session's input delay
   makes the earliest schedulable tick, exactly as a human's click is scheduled. One tick of
   observation lag is the floor, and it is the same floor a player has. A command overtaken by
   events — the unit died on the tick between — is rejected, counted, and hashed by the machinery
   `Units::rejected` already provides; staleness is an ordinary input condition, not a fault.

4. **Its commands enter through the command log, stamped with its seat, and are recorded exactly
   like a player's.** Same `Command` struct, same opaque payload encodings the gameplay layer
   defines, same refusal of out-of-order records, same fold into the tick hash. The log records
   seats, not species: nothing in the log or the hash distinguishes an AI's command from a human's,
   because the simulation never reads the distinction and hashing information the simulation
   ignores would invalidate replay comparison over metadata. Which seats were AI-driven is session
   metadata *beside* the log — the sweeps' reporting needs it, the kernel does not.

5. **Replay never consults the AI.** A recorded match replays by feeding the log to a fresh kernel;
   the AI's commands are in the log, so the AI itself is not instantiated, not versioned against,
   and not required to still exist. This is what makes the M6 exit condition hold *by
   construction*: a live run and its replay cannot disagree about the AI because the replay
   contains no AI to disagree with. It is also what makes the AI swappable — improving it, retuning
   it, or replacing it wholesale invalidates no recorded replay, which is the property
   [balance.md §6.3](../design/balance.md#63-the-caveat-that-makes-the-sweeps-honest)'s sweeps
   quietly depend on when they compare runs across AI revisions.

6. **The AI is nevertheless deterministic by construction, and a test enforces it — separately from
   replay.** Recording alone satisfies the exit condition; the sweeps demand more, because a seeded
   match that cannot be re-*run* identically is a sweep that cannot attribute a difference to the
   number that changed. So the AI carries the kernel's own discipline, applied by rule rather than
   by structure:
   - Seeded from the session seed and its seat, through its own generator. It never draws from the
     kernel's streams — the [determinism invariants](../invariants/determinism.md) already forbid
     any consumer outside the state transition, and the AI is outside it.
   - Keyed to the tick counter, never to wall clock, frame rate, or elapsed time. Two hosts that
     tick the same simulation get the same decisions however fast their frames come.
   - Ordered containers wherever order reaches a decision, stable tie-breaking by object
     identifier — the same rules the kernel follows and for the same reason.
   - [ADR 0007](0007-simulation-arithmetic.md)'s textual scan runs over the AI's code, and its
     transcendentals come from `cic-math`. Not because AI arithmetic reaches simulation state — it
     does not — but because a platform `sin` in a target-scoring heuristic makes a seeded sweep
     unreproducible across platforms, which is the same defect one abstraction out.
   - The enforcement is a CI test that runs one seeded match twice, live AI both times, and
     requires identical command logs and identical per-tick hashes. Not the replay machinery — a
     re-run test, because the property under test is the AI's own reproducibility, which replay
     was just defined never to touch.

7. **In a networked match, the AI is hosted by one machine and its commands are relayed like a
   player's.** One peer computes the AI seat's commands and submits them to the session; every
   other peer receives them as ordinary relayed input. There is no cross-machine AI lockstep, no
   requirement that peers carry the AI's code at all, and an AI patch is not a network
   compatibility break. Which peer hosts which AI seat is M7's session negotiation to define.

8. **The AI lives in its own crate, above `cic-sim`.** It depends on the kernel to read snapshots
   and build command payloads; the kernel never depends on it. The dependency direction is the
   proof that decision 1 holds — the same argument [ADR 7002](7002-script-events.md) makes about
   `cic-script`: the sandbox is a property of the graph, not of what the code happens not to call.

## Rationale

**Why not a subsystem inside the kernel.** It is the alternative with real teeth, so it deserves
its full weight. An in-kernel AI gets named, versioned, seeded streams for free; it reads peers
under the registration-order contract; its state is hashed per tick and `first_divergence` names it
when it drifts; and it *cannot* desync, because it is inside the thing the desync would be measured
against. Every one of those is genuine. It loses on four counts, and the first is structural rather
than preferential.

- **It has no legitimate way to act.** A subsystem mutates only itself — the kernel splits its own
  list to make cross-subsystem writes unspellable — so an AI subsystem cannot move a unit. It
  would need either a kernel channel for emitting commands from inside a tick, which is precisely
  the forged-player-command door ADR 7002 declined to open for scripts, or a licence to write to
  `units` directly, which un-decides the invariant that makes per-subsystem hashes attributable.
  Either way the command log stops being the complete input record: what happened in a match is no
  longer the log, it is the log *plus whatever the AI decided*, and a replay must re-run the AI —
  this exact AI, this exact version, forever — to mean anything.
- **The AI's code becomes part of the state transition.** Every tuning pass is then a determinism
  version bump: a recorded replay is invalid the moment a weight changes, and the sweeps can never
  compare two AI revisions on one recorded baseline. The harness the sweeps need is the opposite —
  swappable opponents against stable recordings.
- **It proves less.** M6 scopes the AI as the thing that exercises every mechanic unattended, and
  the mechanics' front door is the command pipe. A host-side AI drives spawn, move, and everything
  after through the same verbs, the same ownership checks, and the same rejection counting a human
  drives — the harness tests the game. An in-kernel AI drives a private seam no human uses, and
  the skirmish it wins says nothing about the pipe a player will actually push on.
- **It multiplies across the network.** Inside the kernel, the AI runs in lockstep on every peer,
  so every peer must carry bit-identical AI code and an AI patch is a compatibility break for
  people who only wanted to play against each other. Hosted on one machine, it is one seat's input
  source and nobody else's problem.

**Why not the hybrid** — a kernel-side subsystem that reads peers during the tick and emits
commands *for the next tick* through a purpose-built channel. It concedes the command-pipe argument
and keeps the free hashing, and it is the host-side design with extra steps: the emission channel
must exist, must be recorded, and must be replayable, at which point the AI's commands are in the
log and the AI could have lived outside — except now its bugs are desyncs instead of bad play, and
its every revision still invalidates replays because its *reads* are inside the claim.

**Why not recorded-but-undisciplined** — an AI whose commands are logged, satisfying decision 5,
with no reproducibility requirement of its own. Replay genuinely does not care; this is the cheapest
correct answer to the exit condition read narrowly. It is rejected because
[balance.md §6.3](../design/balance.md#63-the-caveat-that-makes-the-sweeps-honest) is not narrow:
a sweep re-run against a changed number must attribute the difference to the number, and it cannot
if the opponent also rolled differently. The discipline costs a seeded generator, ordered
containers, and a scan that already exists — and buys re-runnable matches and a re-run test that
catches a wall-clock dependency the day it is introduced rather than the month the sweeps go noisy.

**Why the re-run test rather than replay-compares-reissue.** The alternative enforcement was: replay
instantiates the AI, lets it re-decide, and compares its re-issued commands against the log. It
tests the same property and it is rejected for coupling — it makes every replay depend on the AI
version that recorded it, which decision 5 just spent its whole argument avoiding, and it turns
"replay a match" into "replay a match with the right AI installed". The property is real; the place
to test it is a dedicated CI job, not the replay path.

## Consequences

- The M6 exit condition's replay half is satisfied by construction at this seam: the AI's commands
  are inputs, inputs are recorded, and a replay is the record. What remains to prove in M6 is
  everything else — that the mechanics themselves stay deterministic under a full match's load.
- The kernel needs nothing. No new trait, no new channel, no registration change — `Kernel::advance`,
  `Kernel::subsystem`, and `CommandLog` are already the whole interface. The terrain viewer's
  standing orders are the existence proof, already running.
- A new crate (`cic-ai` or successor) joins the graph above `cic-sim`, carrying ADR 0007's textual
  scan and a dependency on `cic-math`. The scan's scope grows beyond simulation state for the first
  time, and the reason — sweep reproducibility across platforms — should travel with it.
- **The AI's determinism is a discipline, not a structure, and nothing in the kernel enforces it.**
  This is the unwelcome one, stated plainly. Inside the kernel, a hashed container in the AI would
  be caught by per-subsystem hashes on the tick it mattered; outside, `first_divergence` on two
  live runs of one seed reports only that the *inputs* differed on some tick, and from there the
  hunt is the AI's own problem with no attribution machinery to shorten it. The re-run test makes
  the defect visible in CI; nothing makes it visible in the code. If that hunt happens twice, the
  argument for structural help — a deterministic-AI harness trait, an instrumented decision log —
  should be reopened with this paragraph as its charter.
- Until fog of war lands, the harness AI reads the whole world, and the sweeps inherit that: an
  omniscient opponent plays a different game than a fogged one, so sweep results recorded before
  fog are not comparable with results after. §6.3 already forbids treating the sweeps as verdicts;
  this adds a discontinuity to the reasons.
- Session metadata gains a fact the log deliberately omits — which seats were AI-driven, and which
  AI revision. Where that lives is M7's session and replay-container format to define; the log
  itself stays seat-only per decision 4.
- A saved match resumed mid-run gives the AI amnesia: its plan is host state, not kernel state, so
  it is not snapshotted and not restored. The AI must re-derive its posture from the snapshot it
  wakes to — acceptable for a test harness, and recorded here so a future difficulty pass does not
  discover it as a bug.

## What is left open

- **Difficulty and per-faction competence.** [balance.md
  §6.3](../design/balance.md#63-the-caveat-that-makes-the-sweeps-honest) already states the caveat
  this record inherits and does not solve: a harness AI will play Concord's schedule better than
  Meridian's opportunism, so the sweeps it drives are a signal and never a verdict. Difficulty
  levels, per-faction behaviour authoring, and what "good enough to exercise every mechanic" means
  per mechanic are the AI's own design work, out of scope here.
- **The decision architecture.** [ADR 3001](3001-pathfinding.md)'s consequences note already
  expects a data-driven layer — behaviour trees authored per faction rather than hard-coded — and
  reserves the grid's query surface as a consumer-facing API for it. Nothing in this record
  constrains the architecture beyond decision 6's discipline.
- **The visibility view.** What per-seat query surface fog of war exposes — to the interface and
  therefore, per decision 2, to the AI — is fog's record to write. This one only requires that the
  AI read *that* rather than the omniscient snapshot once it exists.
- **The re-run test's harness.** Decision 6's test needs a headless match runner — kernel, AI, no
  window — which M6's skirmish work and §6.2's sweeps both need anyway. Whichever lands first
  carries it.
- **Which peer hosts an AI seat in a lobby**, and how the session negotiates a host's departure
  mid-match. M7's session design owns both; this record only fixes that the answer is "one machine
  computes the seat, everyone else receives relayed input".
