//! A deterministic, sandboxed scripting language for content that runs inside the simulation.
//!
//! # What is here
//!
//! - [`real`] — the arithmetic ADR 0007 permits, and the transcendentals written to stay inside it.
//! - [`value`] — what a script can hold, and why there is no heap.
//! - [`parse`] — source to a syntax tree, with every limit caller-supplied.
//! - [`compile`](mod@compile) — syntax tree to bytecode, where every name is resolved.
//! - [`vm`] — the interpreter, and the fuel that bounds it.
//! - [`host`] — what the engine offers, as a closed set.
//!
//! # The arithmetic is not this crate's decision
//!
//! [ADR 0007](../../../docs/adr/0007-simulation-arithmetic.md) settled it for the whole engine: `f64`,
//! restricted to the operations IEEE-754 requires to be correctly rounded, no platform transcendental,
//! angles as turns. A script runs inside the simulation, so it inherits all of it — see [`real`].
//!
//! A script and the simulation kernel have to reach the same answer on the same two numbers. That is
//! why this crate does not get to have an arithmetic of its own, and an earlier draft that gave it one
//! is written up in [ADR 7001](../../../docs/adr/7001-scripting-language.md) as a mistake.
//!
//! # Why a language of this project's own
//!
//! Three constraints rule out every obvious alternative, and each is load-bearing rather than a
//! preference. ADR 7001 records the full comparison; the summary:
//!
//! **Data may not name an action the engine does not define, and must fail at load.** The same rule the
//! interface layer's action set enforces. Lua, Rhai and WebAssembly all resolve calls at *run* time, so
//! `sys.grant_resources(...)` in a downloaded mod fails when a player triggers it. Here it is a
//! **compile** error naming the file and the line — see [`host::Interface`].
//!
//! **The restriction has to be enforceable, not merely stated.** A general-purpose language's sandbox is
//! subtractive: you remove what is dangerous and re-audit on every version bump. Here the bytecode simply
//! has no instruction for a forbidden operation, so a script cannot reach one whatever anybody writes.
//!
//! **`unsafe_code` is forbidden at workspace scope.** Lua through `mlua`, and every other C library
//! binding, is FFI and therefore unsafe.
//!
//! # What it costs, stated plainly
//!
//! No lists, no maps, no string building, no closures, no user-defined types. A language with a heap
//! needs a garbage collector, and a collector inside a simulation tick is an allocation order to be
//! non-deterministic about and a pause to be surprised by. The omissions are recorded in
//! [the milestone](../../../docs/milestones/m10-scripting.md) with what would have to change to lift
//! them.
//!
//! # A script cannot hang the match or take the process down
//!
//! Two bounds, and they cover different things. **Fuel** bounds time: every instruction costs one unit,
//! so `while true {}` in a mod is an error naming the line rather than a hung process on every machine
//! in the match at once. **The absence of a heap** bounds space. Between them, running content nobody
//! reviewed inside a simulation tick is a defensible thing to do.
//!
//! Every failure is a value. There is no `unwrap` in the interpreter and no arithmetic that can
//! overflow or produce a non-finite result unreported, because a panic mid-tick takes the match with it.
//!
//! # Two enforcement mechanisms
//!
//! ADR 0007 decision 8 requires a textual test scanning for the forbidden names, since `cargo build`
//! will not. This crate carries one, in `tests/arithmetic_restriction.rs`, and it caught a real
//! violation on its first run. The stronger mechanism is structural and applies to the language rather
//! than to the Rust: there is no opcode for a transcendental, so a script cannot call one.
//!
//! # Example
//!
//! ```
//! use cic_script::{Interface, Limits, RuntimeLimits, StandardHost, Value, Vm, compile};
//!
//! // The engine declares what it offers. Nothing else is reachable.
//! let mut interface = Interface::standard();
//! interface.declare_event("tick", 1)?;
//!
//! let program = compile(
//!     r#"
//!     fn distance(ax, ay, bx, by) {
//!         let dx = bx - ax;
//!         let dy = by - ay;
//!         return sys.sqrt(dx * dx + dy * dy);
//!     }
//!
//!     on tick(elapsed) {
//!         if distance(0, 0, 3, 4) > 4.5 {
//!             sys.log("out of range");
//!         }
//!         // Angles are turns: a quarter turn, not pi over two.
//!         let up = sys.sin(0.25);
//!         return elapsed * 2;
//!     }
//!     "#,
//!     &interface,
//!     Limits::DEFAULT,
//! )?;
//!
//! let mut host = StandardHost::new();
//! let mut vm = Vm::with_limits(RuntimeLimits::DEFAULT);
//! let result = vm.run(&program, &mut host, "tick", &[Value::Int(21)])?;
//!
//! assert_eq!(result, Value::Int(42));
//! assert_eq!(host.log, vec!["out of range".to_owned()]);
//!
//! // A verb the engine does not define fails to compile, rather than failing when it is triggered.
//! assert!(compile("on tick(t) { sys.grant_resources(9999); }", &interface, Limits::DEFAULT).is_err());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod compile;
pub mod host;
pub mod parse;
pub mod real;
pub mod value;
pub mod vm;

pub use compile::{Program, compile, compile_ast};
pub use host::{Host, HostCall, HostError, Interface, InterfaceError, Signature, StandardHost};
pub use parse::{Ast, CompileError, Limits};
pub use real::{ArithmeticError, cos_turns, sin_turns};
pub use value::{Value, ValueError};
pub use vm::{RuntimeError, RuntimeFault, RuntimeLimits, Vm};
