# M10: Scripting

Behaviour in data: a deterministic, sandboxed language for scenario logic, campaign missions, and
whatever a mod wants to do that placements cannot express.

**Status:** Charter met as a language and a machine. What is outstanding is the half that cannot exist
yet — the host verbs a simulation kernel would supply, since [M5](m5-simulation.md) has not been built.

## Charter

- A **language** content authors write by hand and review in diffs. **Done** — see
  [the specification](../formats/script.md).
- **Deterministic to the bit**, because scripts run inside a lockstep simulation. **Done**, by
  inheriting [ADR 0007](../adr/0007-simulation-arithmetic.md)'s arithmetic exactly rather than
  inventing a second one.
- **Sandboxed**: a script cannot reach anything the engine did not offer it. **Done**, and at compile
  time rather than at call time.
- **Bounded**: a script cannot hang the match or exhaust memory. **Done** — fuel bounds time, and the
  absence of a heap bounds space.
- **Diagnostics an author can act on**: a line number and what was expected. **Done**, at both compile
  and run time.
- **Host verbs for the simulation.** **Not done, and not yet possible** — see [Remaining](#remaining).

## Landed

- **`cic-script`**, depending on nothing at all. Not on `serde`, not on a parser generator, not on a
  maths crate — the transcendentals are written here because the platform's are the thing ADR 0007
  forbids.
- **The arithmetic is ADR 0007's, unchanged**: `f64`, restricted to the operations IEEE-754 requires to
  be correctly rounded, with no platform transcendental. A script and the simulation kernel must reach
  the same answer on the same two numbers, and the only way to guarantee that is for them to be doing
  the same arithmetic. See [ADR 7001](../adr/7001-scripting-language.md), including what its first draft
  got wrong.
  - **`sqrt` is used directly**, because the standard *requires* it to be correctly rounded — unlike the
    transcendentals, which it only recommends. That distinction is the whole of the divergence risk.
  - **`sin` and `cos` are written in the permitted set** and take **turns** rather than radians, per ADR
    0007 decision 5. The reduction is then `x - x.floor()`, which is exact for every `f64`, so
    `sys.sin(1e12 + 0.125)` returns the identical bit pattern as `sys.sin(0.125)`.
  - **The series agrees with the platform to the last bit** on the same first-quadrant argument, across
    257 sample points. The few units in the last place that show up against a *whole-angle* reference
    are the reference's own range reduction, which is the problem turns remove.
  - **A non-finite result is a fault**, not because an infinity is non-deterministic — it is not — but
    because every comparison against a NaN is false, so a rule involving one silently does not fire.
  - **Integers stay integers.** `f64` is exact only to 2^53, so whole-number counting uses `i64` with
    checked arithmetic that reports its own overflow.
- **No heap.** Every value is a fixed-size scalar and a string is an index into the program's constant
  table. No allocator in the interpreter, so a script cannot exhaust memory however hostile it is; no
  garbage collector, so no pause and no allocation order to be non-deterministic about.
- **A closed host surface**, which is [the interface layer's action-set rule](../formats/ui-layout.md)
  one layer down. A script may call what it declares and what an `Interface` names, and nothing else.
  `sys.grant_resources(...)` in a downloaded mod is a **compile** error naming the file and the line,
  and the diagnostic lists what *was* available.
  - **Events are closed the same way.** `on tikc(...)` fails to compile. Without that, a misspelled
    handler is a handler that silently never runs, which is indistinguishable from one whose body is
    wrong.
  - **Arity is checked at compile time** for both, so a call with the wrong number of arguments cannot
    reach a player.
- **Fuel**, which is what makes running content nobody reviewed inside a simulation tick defensible.
  Every instruction costs one unit of a caller-supplied budget, so `while true {}` is an error naming the
  line rather than a hung process on every machine in the match at once. Charged per instruction rather
  than per statement, because statements are the unit an *author* thinks in and an attacker does not.
  - `fuel_used` is reported, because a designer tuning a handler against the limit needs to know how
    close it is and "it worked on the test map" is not a budget.
- **Recursion is bounded by a call-depth limit**, not by the native stack. Frames are heap allocated
  precisely so the limit can be checked rather than hit — a native stack overflow is an abort with no
  diagnostic that nothing above can catch.
- **Parser nesting is bounded**, for the same reason and with the same lesson the layout format
  recorded: recursive descent turns expression nesting into call nesting, and four thousand open
  parentheses in a mod file overflow the stack.
- **ADR 0007 decision 8's textual guard**, in `tests/arithmetic_restriction.rs`, scanning this crate's
  shipped source for the forbidden names. **It caught a real violation on its first run** — a `powi` in
  the interpreter's own bounds check — which is the best possible argument for a test that otherwise
  reads like bookkeeping. A test module may still use the platform as an *oracle*, exactly as ADR 0007
  uses `libm`, so the scan stops at `#[cfg(test)]`.
  - **The stronger enforcement is structural.** A script cannot reach a forbidden operation whatever
    anyone writes, because the bytecode has no instruction for one. The Rust side gets a tripwire; the
    language side gets a guarantee.
- **Every failure is a value.** No `unwrap` in the interpreter and no arithmetic that can overflow,
  because this runs inside a tick and a panic mid-tick takes the match with it. Even malformed bytecode
  — a compiler bug rather than a script one — is reported rather than panicked on.
- **The machine keeps nothing between runs.** Anything a script remembers lives on the host side of a
  host function, where it is simulation state that is hashed and replayed with everything else. A script
  with hidden globals would be simulation state the desync report cannot see.
- **Diagnostics carry a line at both ends.** A compile error names what was expected and what was found;
  a runtime fault names the function and the line. Expressions are attributed to the statement
  containing them, which is the granularity an author reads.

## Remaining

- **The host verbs a scenario actually needs** — spawn, order, count, query a zone, set an objective,
  show a briefing. Every one of them is a call into a simulation kernel that does not exist yet, so this
  is blocked on [M5](m5-simulation.md) rather than deferred. The seam is ready: a kernel declares them
  on an `Interface` and implements one trait.
- **Scripts in the map package.** The [package format](../formats/package.md) has no entry for them yet,
  and adding one is a format change rather than a language one.
- **One implementation of the transcendentals, not two.** **Done, ahead of M5.** The implementation
  moved to `cic-math` — Denys's choice of home, 2026-07-29 — a crate below both this one and the kernel
  to come, and this crate now consumes it rather than owning it. The bit-pinning tests moved with the
  code, and both crates carry the decision-8 textual scan, so the property this item guarded — a script
  and the kernel cannot disagree about `sys.sin` — is now structural rather than a warning in a list.
- **A determinism test across platforms.** The property is argued from the standard — only permitted
  operations are used, and the guard proves it — and pinned locally by exact-bit assertions on the
  series and on a whole computed script result. What would settle it is the standard the determinism
  invariants set for the simulation: a recorded run replayed on another platform reproducing the same
  hashes. That needs CI runners on more than one platform and a kernel to hash.

## Exit condition

A scenario's behaviour can be written in a file, loaded from a package, and produce identical results
from identical inputs — checked in CI rather than by hand.

**Partly met.** The language, the sandbox, the arithmetic and the bounds are in and covered by 98 tests.
What is not met is the "loaded from a package" half and the cross-platform half of the determinism
claim, both for the reasons above.

## Design notes

**The language is deliberately dull.** `fn`, `on`, `let`, `if`, `else`, `while`, `return`, the usual
operators, and calls. Every feature omitted is one that cannot interact surprisingly with determinism,
the sandbox, or the bounds — and it is read by people writing mission logic, not by people who enjoy
languages.

**There is no truthiness.** Only a `bool` is a condition; not zero, not nil, not the empty string.
Coercion is a class of bug where a value of the wrong type takes a branch instead of being reported.

**Mixed arithmetic promotes toward the real**, so `2 * 1.5` is `3.0`, while two integers stay an
integer. Truncating the other way would discard a fraction an author wrote deliberately.

## Explicitly not done

- **No lists, maps, or string building.** The largest omission and the one most likely to be revisited.
  A collection needs a heap and a heap needs a collector, and a collector inside a simulation tick is a
  pause to be surprised by and an allocation order to be non-deterministic about. What would lift it
  without a collector is a bounded arena of fixed-capacity collections that cannot nest — enough for
  "the units in this zone" and short of anything needing to trace a cycle.
- **No closures, no user-defined types, no modules.** Each is a language feature with no scenario
  currently asking for it.
- **No `for` loop.** `while` with an explicit counter is one construct instead of two, and a fuel-metered
  interpreter makes the loop shape a matter of taste rather than of cost.
- **No debugger and no breakpoints.** `sys.log` and a line number in every fault. A stepping debugger is
  a large amount of machinery, and the failure it addresses is one that a bounded, side-effect-free
  language already makes rare.
- **No incremental or persistent compilation.** A script is compiled when it loads. Compilation is
  microseconds and the complexity of caching it would exceed the saving.
