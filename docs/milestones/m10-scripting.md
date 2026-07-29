# M10: Scripting

Behaviour in data: a deterministic, sandboxed language for scenario logic, campaign missions, and
whatever a mod wants to do that placements cannot express.

**Status:** Charter met as a language and a machine. What is outstanding is the half that cannot exist
yet — the host verbs a simulation kernel would supply, since [M5](m5-simulation.md) has not been built.

## Charter

- A **language** content authors write by hand and review in diffs. **Done** — see
  [the specification](../formats/script.md).
- **Deterministic to the bit**, because scripts run inside a lockstep simulation. **Done**, by having no
  floating-point type at all.
- **Sandboxed**: a script cannot reach anything the engine did not offer it. **Done**, and at compile
  time rather than at call time.
- **Bounded**: a script cannot hang the match or exhaust memory. **Done** — fuel bounds time, and the
  absence of a heap bounds space.
- **Diagnostics an author can act on**: a line number and what was expected. **Done**, at both compile
  and run time.
- **Host verbs for the simulation.** **Not done, and not yet possible** — see [Remaining](#remaining).

## Landed

- **`cic-script`**, depending on nothing at all. Not on `serde`, not on a parser generator, not on a
  maths crate — the fixed-point routines are here because the platform's are the problem being solved.
- **No floating-point type**, which is the decision the rest follows from and is not a preference. IEEE
  754 says nothing about `sin`, `cos`, `sqrt` or `pow`, so two platforms return different values for the
  same input and both conform; fused multiply-add rounds once where two operations round twice; x87
  computes at 80 bits and rounds when it spills. Each is survivable in presentation and each is a desync
  in a simulation. The full comparison against Lua, Rhai, WebAssembly and a JSON trigger table is in
  [ADR 7001](../adr/7001-scripting-language.md).
  - **Fixed point is an `i64` with 16 fractional bits**: about ±140 trillion, resolving 0.000015. The
    range is what the 64 bits are for — a distance calculation squares its operands, and a 32-bit fixed
    type overflows squaring anything past about 180.
  - **Every operation is checked.** Overflow is an error, not a wrap and not a saturation: wrapping puts
    a unit at the edge of the map at the other edge, saturating turns a runaway calculation into a
    plausible number that is wrong, and both are silent.
  - **`sqrt` is integer Newton iteration and `sin` is an integer polynomial**, so a script computing a
    facing gets the same answer on every machine. Five Taylor terms rather than four, because four would
    have left the approximation as the limiting error by a factor of ten — measured, and written up in
    the ADR.
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
- **A determinism test across platforms.** The property is argued structurally — there is no float, so
  there is nothing to diverge — and the tests pin that the interpreter is stateless and the arithmetic
  is pure. What would settle it is the same standard the determinism invariants set for the simulation:
  a recorded run replayed on another platform reproducing the same state hashes. That needs CI runners
  on more than one platform and a kernel to hash.

## Exit condition

A scenario's behaviour can be written in a file, loaded from a package, and produce identical results
from identical inputs — checked in CI rather than by hand.

**Partly met.** The language, the sandbox, and the bounds are in and covered by 90 tests. What is not
met is the "loaded from a package" half and the cross-platform half of the determinism claim, both for
the reasons above.

## Design notes

**The language is deliberately dull.** `fn`, `on`, `let`, `if`, `else`, `while`, `return`, the usual
operators, and calls. Every feature omitted is one that cannot interact surprisingly with determinism,
the sandbox, or the bounds — and it is read by people writing mission logic, not by people who enjoy
languages.

**There is no truthiness.** Only a `bool` is a condition; not zero, not nil, not the empty string.
Coercion is a class of bug where a value of the wrong type takes a branch instead of being reported.

**Mixed arithmetic promotes toward fixed point**, so `2 * 1.5` is 3.0. Every integer the type can hold
is exactly representable, so the promotion loses nothing, while truncating the other way would discard
a fraction an author wrote deliberately.

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
