# ADR 7001: A scripting language of this project's own

- Status: accepted and **implemented**. The language, compiler, and machine are in.
- **An earlier draft of this record argued the opposite of decision 2** and was wrong. See
  [What the first draft got wrong](#what-the-first-draft-got-wrong).

## Context

Scenarios need behaviour, not just placements. A briefing fires when a zone is entered; reinforcements
arrive when a structure falls; a campaign mission has victory conditions that are not "destroy
everything". Compiling that into the engine means a rebuild for every content change and a content
author who cannot work without a Rust toolchain, so it belongs in data.

Writing a language is a large thing to do and the burden of proof is on doing it. Four constraints
already in force decide it, and none of them can be added to an existing implementation afterwards.

**[ADR 0007](0007-simulation-arithmetic.md) pins the arithmetic.** Scripts run inside the simulation, so
they inherit it exactly: `f64`, restricted to the operations IEEE-754 requires to be correctly rounded,
no platform transcendental, angles as binary turns. The requirement is not "be deterministic" — it is
"only these operations, and prove it".

**Data may not name an action the engine did not define, and must fail at load.** The rule [the
interface layer's action set](../../crates/cic-ui/src/action.rs) already enforces, for the reason
recorded there: a name looked up at call time defers a typo to the moment a player triggers it, and once
mods can supply content a string is an open channel into whatever the lookup table holds.

**A script must not be able to hang the match or exhaust memory.** It runs inside a tick, on every
machine in the match at once.

**`unsafe_code` is forbidden at workspace scope.**

## What was considered

| Candidate | Why not |
|---|---|
| **Lua via `mlua`** | FFI, so `unsafe`. Its sandbox is *subtractive* — you remove globals until nothing dangerous remains — which is an open surface narrowed by hand rather than a closed one opened by hand, and it has to be re-audited on every version bump. Function resolution is at call time, so an unknown name is a runtime error. |
| **Rhai** | Pure safe Rust, and its `f64` arithmetic is correctly rounded, so ADR 0007's *permitted* operations are fine. It fails on the rest: its standard library and exponent operator reach unpermitted operations and would have to be removed rather than never added; calls resolve at run time, so a mod naming a verb the engine lacks fails when triggered rather than when loaded; and it allocates — strings, arrays, maps — so a script inside a tick brings an allocator and the collector question with it. |
| **WebAssembly (`wasmtime`)** | **The strongest alternative, and it got stronger under ADR 0007**: core Wasm has no transcendental instructions at all, and its arithmetic and `f64.sqrt` are correctly rounded, so the permitted set is close to Wasm's instruction set by construction. Ruled out on shape rather than correctness — a very large dependency for a scenario trigger, and a content author editing a mission would need a separate toolchain to *produce* a module. Worth revisiting if scripting ever grows past what people write by hand. |
| **A trigger table in JSON** | No new language, and it is what many RTS engines do. It fails the moment a condition needs arithmetic: the format grows an expression syntax, then variables, then conditionals, and arrives at a language nobody designed. |

## Decision

1. **A small language of this project's own**, compiled to bytecode and run by an interpreter in
   [`cic-script`](../../crates/cic-script/).
2. **The arithmetic is ADR 0007's, unchanged.** `f64` values, only the correctly-rounded operations,
   `sqrt` used directly because the standard requires it to be exact, and sine and cosine written in
   the permitted set — originally in this crate's own `real` module, and since extracted to
   [`cic-math`](../../crates/cic-math/src/lib.rs); see the note at the end of this record.
3. **Angles are turns**, per ADR 0007 decision 5. A script writes `sys.sin(0.25)` for a quarter turn.
4. **No heap.** Every value is a fixed-size scalar; a string is an index into the program's constant
   table. No allocator in the interpreter, no garbage collector, no collection pause, no allocation
   order to be non-deterministic about.
5. **A closed host surface, resolved at compile time.** A script may call the functions it declares and
   the host functions an `Interface` names. `sys.grant_resources(...)` in a downloaded mod is a
   **compile** error naming the file and the line. Events are closed the same way.
6. **Fuel.** Every instruction costs one unit of a caller-supplied budget, so `while true {}` is an
   error naming the line rather than a hung process.
7. **A non-finite result is a fault.** Not for determinism — IEEE-754 pins infinity and NaN as tightly
   as anything else — but because every comparison against a NaN is false, so a rule involving one
   silently does not fire and the author sees nothing at all.

## Rationale

**Decision 5 is what a general-purpose language cannot give.** Every candidate above resolves calls at
run time, so the closure has to be built by *removing* things. A from-scratch compiler resolves every
name against a declared interface before the program can run, and the diagnostic lists what was
available. That is the same argument the interface layer made about its action set, and it is worth a
language on its own.

**Decision 2 is enforced twice, and the second mechanism is stronger than the kernel's own.** ADR 0007
decision 8 needs a textual test scanning Rust source, because `cargo build` will not enforce the
restriction. This crate carries that test — and on its first run it caught a real violation, a `powi`
in the interpreter's own bounds check. But a *script* cannot reach a forbidden operation at all,
whatever anyone writes, because **the bytecode has no instruction for one**. The restriction is
structural on the language side and textual only on the Rust side, which is a better position than the
simulation kernel itself can occupy. That is an argument for putting content behind a VM rather than an
argument about which VM.

**Decision 4 is what makes decision 6 sufficient.** Fuel bounds time. Without a heap there is nothing to
bound in space, and the two together are what "safe to run untrusted content inside a tick" has to mean.
A language with collections needs a collector, and a collector inside a tick is a pause to be surprised
by.

**The language is deliberately dull.** `fn`, `on`, `let`, `if`, `else`, `while`, `return`, the usual
operators, and calls. No closures, no user types, no modules, no truthiness. It is read by people
writing mission logic, and every feature omitted is one that cannot interact surprisingly with the four
constraints.

## What the first draft got wrong

Recorded rather than quietly edited, because the mistake is instructive and the corrected reasoning is
weaker than the original claimed to be.

**The first draft used fixed point**, on the argument that "pinning `f64` across platforms is not
achievable". That is false, and ADR 0007 — written independently, in a branch open at the same time —
says exactly why: IEEE-754 *requires* correct rounding for `+ - * / sqrt fma`, so two conforming
platforms cannot disagree about them. Only the transcendentals are unspecified, which is a far narrower
problem than "floating point is not deterministic".

ADR 0007 also rejects fixed point directly, and every clause applies here:

> It is rejected because it buys determinism that IEEE-754 already guarantees, and charges for it
> everywhere: a conversion at every boundary … and a hand-rolled type with its own overflow behaviour.
> Worse, it does not solve the actual problem — a fixed-point `sin` still has to be written.

There is a fifth cost specific to a script: **two arithmetics inside one simulation.** With the kernel
on `f64` and scripts on fixed point, every value crossing the boundary converts, and a script could
reach a different answer than the kernel would on the same two numbers — in a comparison, on a tick, in
a lockstep match. The correct position is not that a script *may* use the kernel's arithmetic but that
it **must**.

**What survived the correction:** the closed compile-time surface, the absent heap, the fuel, and the
no-panic bar. Those were always the load-bearing reasons for a language of this project's own. The
determinism argument was doing less work than the draft claimed, and it is the one that had to be
narrowed rather than the ones that had to be defended.

**How it was caught:** by hand, during review, because two branches had both allocated ADR number 0007
and the collision forced somebody to read both. That is not a mechanism, which is why ADR numbering
changed at the same time — see [the index](README.md).

## Consequences

- **No lists, maps, or string building.** The largest cost and the one most likely to be revisited. A
  bounded arena of fixed-capacity, non-nesting collections would restore most of the utility without a
  collector; recorded in [M10](../milestones/m10-scripting.md) rather than left implied.
- **The simulation kernel must supply the host verbs**, and each is a deliberate edit in one place.
  That is the property being bought, not a friction to route around.
- **The transcendentals now exist twice in prospect.** This crate has `sin_turns` and `cos_turns`, and
  ADR 0007 decision 4 says the simulation crate supplies its own. When M5 lands they must not become two
  implementations that can disagree — whichever exists first is the one both use, and the natural home
  is a crate below both. Flagged in M10 as the one thing that must not be got wrong twice.
- **A script's state lives on the host side.** The machine keeps nothing between runs, so anything a
  script remembers is simulation state that is hashed and replayed with everything else.
- **WebAssembly remains the escape hatch**, and the `Interface`/`Host` split is the seam it would arrive
  at: a Wasm module implementing the same host contract needs no change above it.

## What implementing it established

**The transcendentals did not stay in this crate, and decision 2 is stronger for it.** They were written
here because this is where they were first needed, and extracted to **`cic-math`** once a simulation
kernel was going to want the same functions — one crate below both, depending on nothing, carrying
ADR 0007 decision 8's textual scan with them. The point decision 2 was making is sharpened rather than
contradicted: the arithmetic is not the scripting language's, it is the project's, and a script and a
kernel able to hold two implementations that could disagree is precisely the state the extraction
removes. The exact-bit pins and the platform-oracle comparison moved with the code.

**The textual guard earned its place immediately.** ADR 0007 decision 8 reads like bookkeeping until it
fails. Its first run on this crate found `2f64.powi(63)` in the `floor` bounds check — `powi` is
forbidden not because it is inexact but because its lowering is unspecified, and the fix is to write the
constant out, since 2^63 is exactly representable. Nothing else in the build would ever have objected.

**The turn-based reduction is exact, and it is measurable rather than asserted.** `sin_turns` agrees
with the platform's `sin` **to the last bit** when both are given the same first-quadrant argument,
across 257 sample points. Comparing against the platform on the *whole* angle instead shows a few units
in the last place — and all of that gap is the reference's own range reduction, not the polynomial. So
`sin_turns(1e12 + 0.125)` returns the *identical bit pattern* as `sin_turns(0.125)`, because the
reduction is a subtraction that cannot round. That is ADR 0007 decision 5 demonstrated.

**Eleven Taylor terms rather than a minimax fit**, and the reason is reviewability. Every coefficient is
a stated reciprocal factorial a reader can verify from the series; minimax coefficients are a fitting
tool's output and have to be trusted. The cost is ten multiplications in a function scripts call rarely.

**Constants are interned by bit pattern, not by equality.** `0.0` and `-0.0` compare equal and divide
differently, so folding them into one constant would change what a program means.

**Equality is exact and cannot be anything else.** A tolerance would make `==` non-transitive, so
`a == b` and `b == c` would stop implying `a == c`, and a script's conditions would stop composing.
