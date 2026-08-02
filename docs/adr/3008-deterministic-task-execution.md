# ADR 3008: Deterministic task execution — phases, buffered effects, stable commits

- Status: proposed

## Context

The simulation kernel is deliberately serial today. It advances subsystems in registration order, and
that order is observable: a subsystem registered earlier has already advanced when a later subsystem
reads it, while one registered later has not. The borrow expresses the other half of the contract — a
subsystem mutates only itself and reads its peers immutably — so the hash that moves is the hash of the
state that actually changed.

That model has now carried movement through its first cross-subsystem dependency: `Ground` reads
`Forces`, and `Units` reads `Ground`. It does not yet answer two related requirements M6 is about to
create:

- combat, construction and scripting need to request a change owned by another subsystem without
  acquiring a mutable reference to it; and
- path searches, avoidance, vision, AI evaluation and later economy updates contain independent work
  that should be able to use several processors without making their completion order part of the game.

Simply running the existing `Subsystem::tick` calls in parallel is not an optimisation of the current
contract. It changes which state a peer observes. A whole-tick front/back buffer makes the rule clear but
adds a tick of latency to every dependency, including a structure stamp that movement must respect now.
Shared mutable state behind locks preserves neither attribution nor a deterministic ordering of
conflicting writes.

Direction proposed by Denys on 2026-08-02: define a task manager with buffered data exchange, designed
for low latency and high processor utilisation. An existing game engine's task system was mentioned as a
category of architecture, not as code or constants to consult. This record is a from-scratch statement
of the contract this simulation needs; the provenance rule in `CONTRIBUTING.md` applies unchanged.

## Decision

1. **The simulation schedule and the task executor are separate contracts.** The schedule decides which
   state a task may observe, where barriers stand, and in what order results become state. The executor
   decides only which ready task runs on which worker. Replacing the executor must not move a tick hash.

2. **The schedule is an explicit acyclic graph of phases.** A phase contains work whose inputs have all
   been committed and whose outputs are independent until the next barrier. Dependencies, not incidental
   queue order, place a subsystem in a phase. A cycle is a construction error that names the systems in
   it rather than an order the executor guesses at runtime.

   The first schedule must reproduce the serial kernel's current observations exactly. `Ground` therefore
   commits after the `Forces` state it reconciles, and `Units` after the ground it routes over. Moving an
   edge later is a gameplay change with replay evidence, not a scheduler tuning knob.

3. **Buffering is per dependency boundary, not one swap for the whole tick.** A task reads an immutable
   committed input and writes only to storage reserved for its output. The phase barrier commits those
   outputs and makes them visible to dependent phases in the same tick. A dependency pointing back to an
   already committed phase is necessarily an input to the next tick and is named as such.

   This is the data-exchange double buffer: read storage remains stable while write storage is filled,
   and they change roles at a declared barrier. It is not a second copy of the entire world and does not
   impose a one-tick delay on every peer read.

4. **State still has one owner.** A task may write disjoint portions of its owning subsystem's staged
   state, or emit a typed effect for another subsystem. It may never mutate a peer directly. The target
   subsystem applies effects at its declared commit point, which keeps validation and hashing beside the
   state they govern.

   A script spawning a unit therefore emits a system action rather than forging a player's command, and
   combat killing an object emits a removal effect rather than reaching into `Forces`. Human commands,
   AI commands and system effects remain distinct inputs even when they eventually call the same
   validation routine.

5. **Parallel work produces indexed results and commits them in a stable order.** Scheduling order,
   worker identity and completion time are never keys. Jobs over objects or cells write to result slots
   selected before dispatch; variable-length effects carry a stable source key and local sequence and are
   ordered at the barrier before they are applied. A reduction has an explicit tree or a serial ordered
   fold rather than depending on which partial result arrives first.

6. **Identifier allocation and simulation randomness stay out of unconstrained jobs.** The coordinator
   may allocate deterministic ranges or prepare random inputs before dispatch, and an accepted later
   record may introduce streams derived from stable object identities. A worker may not draw from the
   kernel's shared allocator or a shared random stream in completion order.

7. **A serial executor is the reference implementation and remains supported.** The phase graph,
   buffers, effects and commits land against it first. It is the debugging path, the small-workload path,
   and the oracle a parallel backend is compared with. Parallel execution is an implementation choice,
   not a requirement for a valid kernel.

8. **A parallel backend uses a fixed worker pool and lightweight, scoped jobs.** It creates no operating
   system thread per task or per tick. Workers may use local queues and steal ready work; a thread waiting
   at a barrier helps run eligible work before it parks. Those are executor details and may change without
   touching the simulation API.

   Work is submitted in bounded ranges rather than one job per trivial operation. The minimum useful
   range is measured per workload, because a scheduler that keeps every core busy doing overhead has not
   made the tick shorter.

9. **Frame-critical and background work cannot silently share one unbounded queue.** Simulation phases
   and presentation work needed for the next frame have a bounded latency requirement; asset conversion,
   streaming preparation and tooling do not. An executor used by both exposes lanes or separate pools so
   background work cannot occupy every worker at the instant the next tick becomes ready.

10. **Simulation jobs do not block on I/O, devices or clocks.** They are finite CPU work over inputs the
    phase already owns. A job may spawn scoped child work and join it through the executor; it may not wait
    for a file, socket, GPU or audio device. This preserves both the kernel boundary and the worker pool's
    ability to account for its critical path.

11. **Parallelism is verified by equivalence and justified by measurement.** Every converted workload
    runs the same replay through the serial and parallel executors and requires identical per-tick,
    per-subsystem hashes. The suite varies worker count, range size and deliberate scheduling
    perturbations. Performance evidence reports the serial time, parallel time, barrier wait, queue wait,
    steals and the workload size; utilisation alone is not success.

12. **The executor implementation is chosen after the first real workload is measured.** A maintained
    scoped fork/join pool is the baseline for validating the API and collecting numbers. A custom
    work-stealing executor is warranted only if the baseline cannot meet a demonstrated latency,
    isolation or observability requirement. The simulation-facing contract above is kept either way.

## First adoption sequence

1. Introduce the phase plan, typed effect buffers and serial executor while preserving every existing
   replay hash.
2. Use the effect path for combat's first cross-subsystem consequences and for the first scripting verbs.
3. Convert one measured, naturally partitioned workload — path requests, avoidance pair ranges or vision
   source ranges — to indexed jobs and require serial/parallel replay equivalence.
4. Measure a general scoped pool before deciding whether a custom backend buys anything.
5. Expand parallel coverage only where the tick profile names useful work; do not split subsystems merely
   to demonstrate the scheduler.

Package-integrated templates do not depend on this sequence and should land first, closing the existing
gap between the generated demonstration and a real `.cicmap`.

## Consequences

- Registration order stops being the only representation of dependencies. The transition must keep it as
  a compatibility check until the explicit graph has proven the same observations.
- Cross-subsystem writes gain a typed, attributable route without weakening the immutable peer boundary.
- Buffer memory and phase barriers are real costs. The graph should have as few barriers as semantics
  require, not one after every function.
- Some effects intentionally take one tick when their dependency points backwards. That latency is visible
  in the phase declaration instead of emerging from queue timing.
- A nondeterministic work-stealing schedule is acceptable because it cannot decide allocation, random
  draws, reductions or commit order.
- A custom executor remains possible without making the simulation depend on one. Its value must be a
  measured reduction in critical-path latency, not ownership of machinery.

## Rejected

- **Parallel calls to the existing `Subsystem::tick` loop** — changes peer-read semantics and permits no
  safe cross-subsystem result commit.
- **One front/back world swap per tick** — clear, but adds one tick of latency to every dependency and
  duplicates state that does not need buffering.
- **A shared mutable world behind locks** — completion and lock-acquisition order become gameplay, while
  the subsystem hash no longer identifies the owner of a bad write.
- **Job completion order as commit order** — fast and explicitly nondeterministic.
- **A custom scheduler before a measured baseline** — optimises unknown job sizes, contention and latency
  requirements and makes scheduler debugging part of the first gameplay slice.
- **Parallelism as a condition of M6 completion** — M6 requires a deterministic playable match. The serial
  executor is correct even if the measured match never needs more.
