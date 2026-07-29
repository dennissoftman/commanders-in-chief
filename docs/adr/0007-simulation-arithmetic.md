# ADR 0007: Simulation arithmetic — `f64`, a restricted operation set, and our own transcendentals

- Status: accepted

## Context

[The determinism invariants](../invariants/determinism.md) say that "floating-point behaviour is pinned
where it reaches simulation state." That is a requirement, not a rule anybody can follow. M5 needs it to
become one, and [M5's design notes](../milestones/m5-simulation.md) say why it is settled there rather than
in M7: it constrains how every line of M6's gameplay maths is written, and discovering the constraint after
the maths exists means rewriting the maths.

**Why the requirement is unusually strict here.** A lockstep simulation feeds its own output back into its
next input. A one-bit difference on the last tick of a match is invisible; a one-bit difference on the first
tick is a different rounding on the second, a comparison that flips on the fiftieth — *is this unit within
range?* — a different pathfinding branch on the two-hundredth, and by the two-thousandth one client has a
tank the other has destroyed. Error does not average out; it amplifies until it is categorical. So the
question is never "how accurate is this" but "is this bit-identical", and the answer is binary.

**What is actually at risk is narrower than it looks.** IEEE-754 *requires* correct rounding for `+`, `-`,
`*`, `/`, `sqrt`, `fma`, remainder, comparison, and conversions. Two conforming platforms cannot disagree
about those, at either width. What the standard only *recommends* is the transcendental functions — `sin`,
`cos`, `atan2`, `exp`, `ln`, `powf` — and in Rust those call the platform's C library: glibc on Linux, the
UCRT on Windows, Apple's libm on macOS. They differ in the last bits, and they differ identically at `f32`
and `f64`.

## Decision

1. **Simulation state is `f64`.** Chosen for accumulation headroom over a long match, *not* for
   determinism — see the rationale, because the reasoning matters more than the choice.
2. **Only correctly-rounded operations may touch simulation state.** The permitted set is `+ - * /`, `%`,
   `sqrt`, `abs`, `signum`, `copysign`, `min`, `max`, `clamp`, comparison, `round`/`floor`/`ceil`/`trunc`,
   and conversions to and from integers. Every one of those is specified exactly by IEEE-754 or by Rust
   itself.
3. **No platform transcendental appears in simulation code.** `sin`, `cos`, `tan`, `asin`, `acos`, `atan`,
   `atan2`, `exp`, `exp2`, `ln`, `log`, `log2`, `log10`, `powf`, `powi`, `hypot`, `cbrt`, `sinh`, `cosh`,
   `tanh`, and `to_degrees`/`to_radians` are all forbidden there. `powi` is on the list not because it is
   inexact but because its lowering is not specified; write the multiplication.
4. **The simulation crate supplies its own transcendentals**, written in the permitted set only, so they
   are deterministic by construction rather than by trust. Each is pinned by a test asserting exact bit
   patterns, not an approximate comparison — an approximate test for a function whose whole purpose is
   bit-exactness verifies nothing.
   - **`libm` is the oracle, not the implementation.** It is a dev-dependency, used to check our accuracy
     against a battle-tested MUSL port, and it never appears in a build. See *What was looked at first*.
5. **Angles reaching those functions are stored as binary turns** — an unsigned integer fraction of a full
   revolution — rather than as radians. Range reduction is where a naive `sin` loses both accuracy and
   determinism, because reducing a large radian argument modulo π needs more precision than the argument
   has. A turn count wraps exactly, for free, in the integer domain.
6. **`mul_add` is permitted but is not interchangeable with `a * b + c`.** Both are deterministic — `fma`
   is correctly rounded by the standard — but they round differently, so they are different functions.
   Rust never contracts one into the other implicitly, which is what makes this safe; a reviewer must not
   "simplify" between them.
7. **32-bit x86 targets using the x87 stack are out of scope.** Its 80-bit intermediates make the same
   expression round differently depending on when a value spills to memory, which no source-level rule can
   fix. SSE2 or better, which every x86-64 and aarch64 target has.
8. **A test enforces the restriction textually**, because `cargo build` will not. It scans the simulation
   crate for the forbidden names and fails naming the file and the call.
9. **Presentation is unrestricted.** It may use `f32`, `sin`, or anything else, because a frame is not
   state. The conversion runs one way only: simulation `f64` down to render `f32`, once per frame, and
   never back into state.

## What was looked at first

Writing trigonometry is not obviously better than taking someone else's, so the ecosystem was checked
before any of the above was committed to. Three findings, and the third is the one that decided it.

**[`libm`](https://crates.io/crates/libm) is the near-miss, and it is already in this tree.** A pure-Rust
port of MUSL's libm, MIT-licensed, and already in `Cargo.lock` at 0.2.16 by way of `naga` and
`num-traits` — so it is licence-cleared and vendored already. Being pure Rust rather than a call into the
platform's C library, it should compute the same bits everywhere, which is exactly the property wanted.

It is not taken as the implementation for three reasons, in ascending order of weight.

- **It makes no reproducibility claim.** Checked in three places — the repository's own README, the
  published documentation, and the README at its new home — and none of them mentions determinism,
  reproducibility across targets, or correctness of rounding at all. That is not evidence of divergence;
  it is the absence of a promise, and this ADR exists to produce a guarantee rather than a likelihood.
- **Its own test suite records the hazard this ADR excludes.** `sin.rs` carries
  `#[cfg_attr(x86_no_sse, ignore = "FIXME(i586): possible incorrect rounding")]` — a test disabled on
  x86 without SSE because the rounding there may be wrong. That is the x87 80-bit problem decision 7
  already puts out of scope, so it does not apply to any supported target. It is reassuring rather than
  disqualifying: the implementation itself contains no architecture-gated or FMA-gated code path, only
  basic arithmetic and bit manipulation.
- **It was archived in April 2025** and folded into `compiler-builtins`. Depending on the standalone crate
  means depending on a crate that has moved, for a property nobody has written down.

**Fixed-point is what the ecosystem actually reaches for**, and the search confirms it: `cordic`
implements the transcendentals over fixed-point by CORDIC, `simba` lets `nphysics` run on a fixed-point
scalar explicitly for cross-platform determinism, and `iFloat` exists for the same purpose. That
consensus is itself the evidence for the paragraph below — the reason everyone reaches for fixed-point is
precisely that *no float library guarantees the transcendentals*. It is the road not taken here, and
`cordic` is where to start if this ADR is ever reversed.

**And the hazard is recognised as an opt-in rather than a default.** `fastmaths` assumes FMA at compile
time unless a `soft-fma` feature is enabled "for deterministic builds", which is a crate in this space
treating reproducibility as something a caller asks for. This one asks for it by construction.

**What the evaluation changed.** Nothing in the decision, and one thing in the plan: `libm` becomes the
**oracle** rather than the implementation. Exact-bit tests are needed either way — a version bump changing
a polynomial coefficient is as much a desync as a platform difference — so the marginal cost of writing
our own is the implementation and not the verification. The marginal gain is that the implementation
cannot change beneath us. Testing against `libm` gets MUSL's accuracy, validated, without MUSL's silence
about reproducibility.

**And decision 5 is what makes decision 4 affordable.** Writing a correct `sin` is hard almost entirely
because of range reduction: reducing a large radian argument modulo π needs more precision than the
argument carries, which is why MUSL's `rem_pio2` is the longest and most delicate part of its `sin`.
Storing angles as integer turns removes that problem rather than solving it — the reduction is a mask.
What is left to write is a polynomial over a bounded interval, which is ordinary work.

## Rationale

**Why not fixed-point.** It is the traditional answer and it does work. It is rejected because it buys
determinism that IEEE-754 already guarantees, and charges for it everywhere: a conversion at every boundary
with the renderer and the asset formats, a hand-rolled type with its own overflow behaviour, and gameplay
expressions that a reviewer reads through a layer of scaling. Worse, it does not solve the actual problem —
a fixed-point `sin` still has to be written, so the transcendental work happens either way. Paying the
ergonomic cost of fixed-point *and* still writing your own trigonometry is the worst of both.

**Why `f64` rather than `f32`, and why that is not the determinism argument.** `f32` is exactly as
deterministic: the operations in the permitted set are correctly rounded at both widths, and neither width
makes the platform's `sin` portable. Anybody reaching for a wider type to *fix* a divergence is treating a
reproducibility problem as an accuracy problem, and will find that doubling the mantissa moves the
disagreement to a later bit rather than removing it — which is worse, because it takes longer to show up.

The honest argument for `f64` is accumulation headroom. `f32` carries about seven significant digits, and a
position integrated tick by tick over a long match accumulates rounding in the low bits of a number whose
high bits are the map coordinate. It stays *deterministic* while getting grainy, so the failure is a unit
that drifts identically on every client rather than a desync — an easier bug and still a bug. `f64` removes
the class rather than managing it, which is the trade this project has made repeatedly.

The cost is real and accepted: simulation state doubles in size, which reaches M7's snapshots and deltas.
It lands where it hurts least, because simulation state is small beside render data — a map's heightfield
and textures dwarf its unit list — and M7 sends *commands* rather than state.

**Why the restriction is the whole mechanism.** Everything above is bookkeeping around one observation: the
thing that differs between platforms is not the arithmetic, it is the library. Once no library call reaches
simulation state, there is nothing left to be non-deterministic, and the property holds by construction
rather than by testing for its absence.

**Why a textual test rather than a type.** A newtype wrapping `f64` and exposing only the permitted
operations would enforce this at compile time, which is stronger. It is rejected for now because it makes
every gameplay expression pass through operator implementations and turns a readable formula into a wall
of wrapper calls — and because the enforcement it adds is over a mistake a grep catches at the same moment
CI would. If the textual test ever misses one, that is the argument for the newtype and it should be
revisited then.

## Consequences

- Gameplay maths in M6 is written in a restricted dialect of Rust, and this ADR is the reference for what
  is in it. The restriction is stated before the maths exists, which is the entire reason this is decided
  in M5.
- The simulation crate owns a small mathematics module — trigonometry, inverse trigonometry, and whatever
  else gameplay actually needs — each function pinned by exact-value tests. It is more code than calling
  the platform's, and it is code that cannot desync. `libm` is a **dev-dependency** it is checked against,
  which adds nothing to a build; if that ever becomes a runtime dependency, this ADR has been reversed and
  should say so.
- **Angles are integers in simulation state.** That is visible in the state layout, in save files later,
  and in what a designer sees in a scenario, so it is a format decision as much as an arithmetic one.
- The forbidden-call test joins the small family of tests here that check things the compiler does not:
  no orphaned shader chunk, no inherited licence header, no reference image missing.
- Presentation keeps its freedom, which is where the existing rule that presentation must not consume
  simulation randomness already points. The two rules are the same rule from opposite ends: state may not
  depend on the frame, and the frame may do whatever it likes.
- A platform without SSE2 or an equivalent is unsupported. This is a narrowing, and it costs nothing any
  target this project cares about.
- If a desync ever appears despite this, the first question is which forbidden call got through — not which
  number was inaccurate. That is a much shorter search, and it is the return on deciding this here.
- **The cheap reversal is available.** If writing the mathematics module turns out to cost more than it is
  worth, `libm` is already vendored and its implementation contains no architecture-gated path, so
  switching to it is a one-line change plus keeping the same exact-bit tests. That is worth recording
  because it means this decision is not a bet: the fallback is better than the alternative it replaced,
  and the tests that would catch a problem are written either way.

## What implementing it established

Decision 4 says "the simulation crate supplies its own transcendentals", and implementation order made
that phrasing obsolete before the simulation crate existed: M10's script VM needed the same `sin` first,
which is exactly the two-implementations hazard its milestone flagged. So the transcendentals live in
**`cic-math`** — extracted from `cic-script` on 2026-07-29, a crate below every simulation-side consumer
and depending on nothing — and "supplies its own" now means "consumes the shared crate's". The exact-bit
pins and the platform-oracle comparison moved with the code, and decision 8's textual scan is carried by
`cic-math` and by each consumer separately, so the guard travels with the code it guards.
