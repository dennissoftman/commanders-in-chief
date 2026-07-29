# Script (`*.cics`)

Behaviour in data: scenario logic, campaign objectives, and whatever a mod wants to do that placements
cannot express.

A small language of this project's own, compiled to bytecode and run by an interpreter that cannot
allocate, cannot hang, and cannot reach anything the engine did not offer it. Why it is not Lua, Rhai,
or WebAssembly is [ADR 7001](../adr/7001-scripting-language.md); the short version is that all three
resolve calls at run time, so a mod naming a verb the engine lacks fails when a player triggers it
rather than when the file loads.

Its arithmetic is [ADR 0007](../adr/0007-simulation-arithmetic.md)'s, unchanged — the same `f64` and the
same restricted operation set the simulation kernel uses. A script and the kernel have to reach the same
answer on the same two numbers.

## Example

```
// Reinforcements when the forward depot falls, and a briefing the first time it is close.

fn distance(ax, ay, bx, by) {
    let dx = bx - ax;
    let dy = by - ay;
    return sys.sqrt(dx * dx + dy * dy);
}

on tick(elapsed) {
    let range = distance(sys.player_x(), sys.player_y(), 512, 288);

    if range < 60.0 && !sys.flag("depot_warned") {
        sys.set_flag("depot_warned", true);
        sys.briefing("mission.depot.approach");
    }

    if sys.structures_remaining(1) == 0 {
        sys.spawn_wave("aec.relief_column", 96, 480);
    }
}
```

`sys.player_x` and the rest are examples. **The engine decides what exists**; there is no fixed standard
library of game verbs, and a script naming one the engine has not declared fails to compile.

## Types

| Type | Written | Notes |
|---|---|---|
| `int` | `42`, `-7` | 64-bit. Overflow is an error, not a wrap. |
| `real` | `1.5`, `0.25`, `1.5e-3` | `f64`, restricted to ADR 0007's permitted operations. |
| `bool` | `true`, `false` | The only thing a condition accepts. |
| `str` | `"text"` | Immutable, and cannot be built at run time. |
| `nil` | `nil` | What a function without a `return` produces. |

A decimal literal is parsed to the nearest representable value, which Rust specifies exactly — so every
platform turns the same text into the same bits. A literal too large to represent is refused rather than
becoming an infinity.

**Mixed arithmetic promotes toward `real`**: `2 * 1.5` is `3.0`. Two integers stay an integer, so `7 / 2`
is `3` — not an optimisation, but because `f64` is exact only to 2^53 and whole-number counting is better
served by arithmetic that reports its own overflow.

**A result that is not finite is a fault.** Dividing by zero, or overflowing the range, stops the handler
with a diagnostic. IEEE-754 defines both perfectly well and they are still mistakes: every comparison
against a NaN is false, so a rule involving one silently does not fire and nothing reports why.

## Statements

```
let name = expression;          // introduces a local
name = expression;              // assigns to one that exists
if condition { } else { }       // `else if` chains
while condition { }
return expression;              // or `return;`
expression;                     // for its effect
// a comment, to the end of the line
```

A local declared inside a block leaves scope at its end. `let` is required to introduce one; assigning
to a name that was never declared is a compile error, so a typo cannot silently create a variable.

## Declarations

```
fn name(a, b) { ... }           // callable from anywhere in the script
on event_name(a) { ... }        // runs when the engine raises that event
```

Declaration order does not matter — a function may call one written below it.

`on` binds to an event **the engine defines**. `on tikc(...)` is a compile error naming the line and
listing the events that do exist. Without that, a misspelled handler is a handler that silently never
runs, which is indistinguishable from one whose body is wrong. The number of parameters must match the
engine's declaration too.

## Operators

| Precedence | Operators | |
|---|---|---|
| tightest | `-x`, `!x` | unary |
| | `*`, `/`, `%` | |
| | `+`, `-` | |
| | `<`, `<=`, `>`, `>=` | |
| | `==`, `!=` | |
| | `&&` | short circuits |
| loosest | `\|\|` | short circuits |

`&&` and `||` do not evaluate their right operand when the left settles the answer, so
`ready && expensive_check()` is a usable idiom.

**There is no truthiness.** Only a `bool` is a condition — not zero, not `nil`, not the empty string.
`if count { }` is a fault naming the type, because coercion is a class of bug where a value of the wrong
type takes a branch instead of being reported.

`==` compares across the two numeric types, so `1 == 1.0`. Values of unrelated types are simply unequal
rather than an error, so a `nil` check is expressible.

## Host functions

Written `sys.name(...)`. `sys` is not a value and not a keyword; it is the only identifier a dot may
follow, which makes the host surface a closed namespace rather than a field access the language would
have to define.

**A script cannot call anything the engine has not declared.** `sys.grant_resources(9999)` in a
downloaded mod is a compile error naming the file and the line, and the diagnostic lists what *was*
available. There is no reflection, no dynamic lookup, and no module system. This is [the interface
layout's action-set rule](ui-layout.md) one layer down, and the argument is the same: a name looked up
at call time defers a typo to the worst possible moment, and once mods can supply content, a string is
an open channel into whatever the lookup table happens to contain.

Argument counts are checked at compile time too.

### The standard set

Available where a host offers it, and a host may decline. These exist because the platform's versions
are the thing being avoided. IEEE-754 only *recommends* correct rounding for the transcendentals, so two
conforming libraries disagree in the last bits — `sys.sin` is a polynomial in the permitted operations,
so it returns the same value on every machine.

| Function | Returns | |
|---|---|---|
| `sys.abs(x)` | `real` | |
| `sys.min(a, b)`, `sys.max(a, b)` | `real` | |
| `sys.clamp(x, low, high)` | `real` | A reversed range returns `low`. |
| `sys.sqrt(x)` | `real` | A negative argument is a fault. |
| `sys.sin(t)`, `sys.cos(t)` | `real` | **Turns, not radians** — see below. |
| `sys.floor(x)` | `int` | Toward negative infinity, because what it is for is an index. Outside the `int` range it is a fault. |
| `sys.log(text)` | `nil` | Diagnostic only. |

### Angles are turns

One full revolution is `1.0`, so `sys.sin(0.25)` is one and `sys.cos(0.5)` is minus one. A script never
writes pi.

This is [ADR 0007](../adr/0007-simulation-arithmetic.md) decision 5, and the reason is reproducibility
rather than taste. Reducing a large *radian* angle modulo pi needs more precision than the angle carries,
which is the longest and most delicate part of any `sin` implementation and the place two libraries most
easily disagree. In turns the reduction is a subtraction that cannot round, so `sys.sin(1000000.125)` and
`sys.sin(0.125)` return the identical value rather than merely a close one.

It also reads better for what game logic does with angles, which is mostly fractions of a circle.

## What a script cannot do

Not a list of restrictions so much as the shape of the thing. Each is in
[M10](../milestones/m10-scripting.md) with what would have to change to lift it.

- **No lists, maps, or string building.** A collection needs a heap, a heap needs a garbage collector,
  and a collector inside a simulation tick is a pause to be surprised by and an allocation order to be
  non-deterministic about.
- **No closures, user-defined types, or modules.**
- **No `for` loop.** `while` with an explicit counter.
- **No global variables.** The interpreter keeps nothing between runs; anything a script remembers lives
  behind a host function, where it is simulation state that is hashed and replayed with everything else.
  A script with hidden globals would be simulation state a desync report cannot see.
- **No file, network, or clock access**, by construction rather than by a blocklist — the only things
  reachable are the host functions the engine declared.

## Limits

Every one is supplied by the caller, following the convention every decoder in this project uses: an
editor loading a campaign can be generous and a multiplayer client accepting a script from a lobby can
be strict, running identical code.

| Limit | Default | Bounds |
|---|---|---|
| `max_source_bytes` | 1 MiB | Source size. |
| `max_tokens` | 200,000 | Lexer output. |
| `max_depth` | 64 | Expression, statement, and block nesting. |
| `max_functions` | 1,024 | Declarations per script. |
| `max_arguments` | 16 | Parameters or arguments per call. |
| `fuel` | 100,000 | Instructions per run. |
| `max_stack` | 4,096 | Operand stack depth. |
| `max_call_depth` | 64 | Nested calls. |

**`max_depth` is not a style rule.** The parser is recursive descent, so expression nesting is call
nesting, and a file of four thousand open parentheses overflows the native stack — an abort, with no
diagnostic, that nothing above can catch.

**`fuel` is what makes running untrusted content inside a tick defensible.** `while true {}` costs its
budget and stops with an error naming the line, rather than hanging the process on every machine in the
match at once. It is charged per instruction rather than per statement, because statements are the unit
an author thinks in and an attacker does not.

## Diagnostics

Both ends carry a line.

```
line 7: the engine defines no `sys.grant_resources`; it defines `abs`, `min`, `max`, `clamp`, `sqrt`, `sin`, `cos`, `floor`, `log`
line 3: `y` is not declared
line 12: `sys.min` takes 2 arguments, not 1
in `tick` at line 9: division by zero
in `tick` at line 4: ran for more than 100000 instructions
```

A runtime fault names the function and the line. Expressions are attributed to the statement containing
them, since an expression can span several lines and the statement is the granularity an author reads.
