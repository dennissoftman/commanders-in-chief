# ADR 7001: A scripting language of this project's own, and why nothing off the shelf fits

- Status: accepted and **implemented**. The language, compiler, and machine are in.

## Context

Scenarios need behaviour, not just placements. A briefing fires when a zone is entered; reinforcements
arrive when a structure falls; a campaign mission has victory conditions that are not "destroy
everything". Compiling that into the engine means a rebuild for every content change and a content
author who cannot work without a Rust toolchain, so it belongs in data.

Writing a language is a large thing to do and the burden of proof is on doing it. The honest starting
position is that one of Lua, Rhai, WebAssembly, or a data-driven trigger table should be used, and this
ADR exists because three of this project's existing rules rule each of them out — not on preference, but
on a property that cannot be added afterwards.

**Determinism.** Scripts run inside the simulation, so [the determinism
invariants](../invariants/determinism.md) apply: *floating-point behaviour is pinned where it reaches
simulation state, and anything that cannot be pinned stays in presentation.* Pinning `f64` across
platforms is not a matter of care:

- IEEE 754 specifies the five basic operations and says nothing about `sin`, `cos`, `sqrt` or `pow`, so
  two platforms' maths libraries return different values for the same input and both conform. A unit's
  facing computed from an angle differs between a Windows client and a Linux one on the first frame
  anything turns.
- Fused multiply-add rounds once where two separate operations round twice, and whether the compiler
  emits it depends on target, optimisation level, and luck.
- The x87 unit computes at 80 bits and rounds when it spills, so on a 32-bit target a result can depend
  on register pressure.

Every general-purpose embedded language is built on `f64` and the platform's maths library.

**`unsafe_code` is forbidden at workspace scope.** Lua through `mlua` — and every other C library
binding — is FFI, therefore `unsafe`.

**Data may not name an action the engine did not define.** The rule [the interface layer's action
set](../../crates/cic-ui/src/action.rs) already enforces, for the reason recorded there: a name looked
up in a table at call time defers a typo to the moment a player triggers it, and once mods can supply
content, a string is an open channel into whatever the table happens to contain.

## What was considered

| Candidate | Why not |
|---|---|
| **Lua via `mlua`** | FFI, so `unsafe`. `f64` numbers throughout, and `math.sin` is the platform's. Its sandbox is a matter of removing globals — an open surface narrowed by hand rather than a closed one opened by hand. |
| **Rhai** | Pure safe Rust, which clears the second constraint cleanly. Still `f64` arithmetic and standard-library transcendentals, so it fails the first. Its `no_float` feature removes fractions entirely rather than replacing them with something reproducible. |
| **WebAssembly (`wasmtime`)** | Genuinely deterministic for integer and basic float operations, and genuinely sandboxed — the strongest candidate. Ruled out on size and shape rather than correctness: it is a very large dependency for a scenario trigger, it needs a separate toolchain to *produce* a module, and a content author editing a mission would be compiling one. Worth revisiting if scripting ever grows past what content authors write by hand. |
| **A trigger table in JSON** | No new language, and it is what many RTS engines do. It fails the moment a condition needs arithmetic: the format grows an expression syntax, then variables, then conditionals, and arrives at a language nobody designed. |

## Decision

1. **A small language of this project's own**, compiled to bytecode and run by an interpreter in
   [`cic-script`](../../crates/cic-script/).
2. **No floating-point type.** All fractional arithmetic is fixed point — an `i64` with 16 fractional
   bits — so every operation is integer arithmetic and identical on every platform. `sqrt`, `sin` and
   `cos` are implemented on it here, by integer Newton iteration and an integer polynomial.
3. **No heap.** Every value is a fixed-size scalar; a string is an index into the program's constant
   table. No allocator in the interpreter, no garbage collector, no collection pause, no allocation
   order to be non-deterministic about.
4. **A closed host surface.** A script may call the functions it declares and the host functions an
   [`Interface`](../../crates/cic-script/src/host.rs) names. `sys.grant_resources(...)` in a downloaded
   mod is a **compile** error naming the file and the line. Events are closed the same way: `on tikc(...)`
   fails to compile rather than silently never running.
5. **Fuel.** Every instruction costs one unit of a caller-supplied budget. `while true {}` is an error
   naming the line, not a hung process on every machine in the match at once.

## Rationale

**Decision 2 is the one that forces the whole design, and it is not a preference.** Any language with a
float type fails the determinism invariant on its first transcendental call, and a language for a
lockstep RTS that cannot compute a distance reproducibly is not a language for a lockstep RTS. Once
floats are gone, most of the reason to adopt an existing implementation goes with them — what would be
adopted is precisely the arithmetic that has to be replaced.

**Decision 3 is what makes decision 5 sufficient.** Fuel bounds time. Without a heap there is nothing to
bound in space, so the two together are what "safe to run untrusted content inside a simulation tick"
has to mean. A language with collections needs a collector, and a collector inside a tick is a pause to
be surprised by and an allocation order to be non-deterministic about.

**Decision 4 costs a line of host code per verb and removes a class of bug entirely.** Every name is
resolved at compile time, so nothing is looked up while a script runs. The diagnostic lists what *was*
available, which turns a misspelling into a fixed problem rather than a reported one.

**The language is deliberately dull.** `fn`, `on`, `let`, `if`, `else`, `while`, `return`, the usual
operators, and calls. No closures, no user types, no modules, no operator overloading, no truthiness. It
is read by people writing mission logic, and every feature omitted is one that cannot interact
surprisingly with the four properties above.

## Consequences

- **No lists, maps, or string building.** The largest cost, and the one most likely to be revisited. A
  bounded arena of fixed-capacity, non-nesting collections would restore most of the utility without a
  collector; it is recorded in [M10](../milestones/m10-scripting.md) rather than left implied.
- **The simulation kernel must supply the host verbs**, and each is a deliberate edit in one place.
  That is the property being bought, not a friction to route around.
- **Scripts are text, and text is the diffable half of this project's format split** — the same
  reasoning that makes scenarios JSON. The language is specified in
  [docs/formats/script.md](../formats/script.md) so a content author has something to read.
- **A script's state lives on the host side.** The machine keeps nothing between runs, so anything a
  script remembers is simulation state that is hashed and replayed with everything else. A script with
  hidden globals would be simulation state the desync report cannot see.
- **WebAssembly remains the escape hatch**, and the `Interface`/`Host` split is the seam it would arrive
  at: a Wasm module implementing the same host contract would need no change above it.

## What implementing it established

**Four Taylor terms were not enough for sine, and the shortfall was measured rather than assumed.**
Through `x^7` the series is accurate to 1.5 parts in 10,000 at a quarter turn; the representation
resolves 1.5 parts in 100,000. The approximation would have been the limiting error by a factor of ten,
in a routine whose entire purpose is to be more predictable than the platform's. The fifth term costs
one multiplication and takes the error below what the type can represent.

**Fixed-point multiplication has to round rather than truncate, and a polynomial is where it shows.** A
bare arithmetic shift floors, so every multiply is biased the same direction by up to one step. That is
invisible once and accumulates across the nine multiplications the sine series performs. Adding half
before the shift is round-half-up, which is not symmetric about zero — acceptable, because what this
needs is a rule every platform applies identically, and any consistent rule satisfies that.

**The 64-bit representation is load-bearing, and the first attempt to demonstrate why was wrong.** A
test asserted that a million squared overflows; it does not — the range is about ±140 trillion, and the
real ceiling is a value of about 11.8 million. The argument for 64 bits over 32 stands and is now stated
correctly: a 32-bit fixed-point type overflows squaring anything past about 180, and a distance
calculation squares its operands, so on a map measured in the hundreds that is an everyday value rather
than an edge case.

**Nesting had to be bounded before the parser was usable on untrusted input.** Recursive descent makes
expression nesting into call nesting, and four thousand open parentheses overflow the native stack — an
abort, with no diagnostic, that nothing above can catch. The interface layer reached the same conclusion
about its layout format. The limit is caller-supplied and there is a test that builds the pathological
file.
