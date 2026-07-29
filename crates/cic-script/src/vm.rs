//! The interpreter.
//!
//! # Fuel is what makes running untrusted content defensible
//!
//! A script arrives from content, and content arrives from a mod. `while true {}` in a mod is not a
//! hypothetical, and inside a simulation tick it is not a slow frame — it is a hung process with no
//! diagnostic, on every machine in the match at once.
//!
//! So every instruction costs one unit of [`RuntimeLimits::fuel`], and running out is an error naming
//! the function and the line. Combined with the absence of a heap — see [`crate::value`] — that bounds
//! a script in both time and space, which between them are what "safe to run inside a tick" has to
//! mean.
//!
//! Fuel is charged per instruction rather than per statement or per loop iteration, because those are
//! the units an *author* thinks in and an attacker does not. A single expression can be arbitrarily
//! long.
//!
//! # Every failure is a value, never a panic
//!
//! There is no arithmetic in this file that can overflow, no index that is not checked, and no
//! `unwrap`. A script must not be able to take the process down, and the reason is the same one the
//! binary decoders have: this runs inside a tick, and a panic mid-tick takes the match with it.
//!
//! # The interpreter holds no state between runs
//!
//! [`Vm::run`] starts with an empty stack and ends with one. Anything a script needs to remember lives
//! on the host side of a host function, where it is part of the simulation's state and is hashed and
//! replayed with everything else. A script with its own hidden globals would be simulation state that
//! the desync report cannot see.

use crate::compile::{Op, Program};
use crate::host::{Host, HostCall, HostError};
use crate::value::{Value, ValueError, arithmetic, compare, equals, negate};

use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// Bounds on what one run may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeLimits {
    /// Instructions one run may execute.
    pub fuel: u64,
    /// Values the operand stack may hold.
    pub max_stack: usize,
    /// Nested calls permitted.
    pub max_call_depth: usize,
}

impl RuntimeLimits {
    /// Limits sized for a handler running inside a simulation tick.
    ///
    /// A hundred thousand instructions is far more than any reasonable handler and far less than a
    /// frame's budget, so the limit bites on a runaway script and on nothing else.
    pub const DEFAULT: Self = Self {
        fuel: 100_000,
        max_stack: 4_096,
        max_call_depth: 64,
    };
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What went wrong while running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeFault {
    /// An operator was misapplied, or the arithmetic failed.
    Value(ValueError),
    /// A host function refused.
    Host(HostError),
    /// The run used its whole instruction budget.
    OutOfFuel {
        /// The budget it was given.
        limit: u64,
    },
    /// The operand stack grew past its limit.
    StackOverflow {
        /// The limit.
        limit: usize,
    },
    /// Calls nested deeper than the limit, which is what unbounded recursion looks like.
    CallDepth {
        /// The limit.
        limit: usize,
    },
    /// The program does not handle the event that was raised.
    UnhandledEvent {
        /// The event's name.
        name: String,
    },
    /// The event was raised with the wrong number of arguments.
    EventArity {
        /// How many the handler takes.
        expected: u8,
        /// How many arrived.
        found: usize,
    },
    /// The bytecode was not well formed, which is a compiler fault rather than a script one.
    ///
    /// Reported rather than panicked on, because the alternative is that a bug in this crate takes
    /// the whole match down instead of one handler.
    Malformed {
        /// What was inconsistent.
        detail: &'static str,
    },
}

impl Display for RuntimeFault {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(error) => write!(formatter, "{error}"),
            Self::Host(error) => write!(formatter, "{error}"),
            Self::OutOfFuel { limit } => {
                write!(formatter, "ran for more than {limit} instructions")
            }
            Self::StackOverflow { limit } => {
                write!(formatter, "operand stack grew past {limit} values")
            }
            Self::CallDepth { limit } => write!(formatter, "calls nested deeper than {limit}"),
            Self::UnhandledEvent { name } => {
                write!(formatter, "the script does not handle `{name}`")
            }
            Self::EventArity { expected, found } => write!(
                formatter,
                "the handler takes {expected} arguments, but {found} were raised"
            ),
            Self::Malformed { detail } => {
                write!(formatter, "malformed bytecode: {detail}")
            }
        }
    }
}

/// A failure, with where it happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError {
    /// What went wrong.
    pub fault: RuntimeFault,
    /// The function it happened in.
    pub function: String,
    /// The line it happened on, or zero when there is none to name.
    pub line: u32,
}

impl Display for RuntimeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(formatter, "in `{}`: {}", self.function, self.fault)
        } else {
            write!(
                formatter,
                "in `{}` at line {}: {}",
                self.function, self.line, self.fault
            )
        }
    }
}

impl Error for RuntimeError {}

/// One active call.
#[derive(Debug, Clone, Copy)]
struct Frame {
    function: u16,
    ip: usize,
    base: usize,
}

/// The interpreter.
#[derive(Debug, Clone, Default)]
pub struct Vm {
    stack: Vec<Value>,
    frames: Vec<Frame>,
    limits: RuntimeLimits,
    fuel_used: u64,
}

impl Vm {
    /// A machine with the default limits.
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(RuntimeLimits::DEFAULT)
    }

    /// A machine with the given limits.
    #[must_use]
    pub fn with_limits(limits: RuntimeLimits) -> Self {
        Self {
            stack: Vec::new(),
            frames: Vec::new(),
            limits,
            fuel_used: 0,
        }
    }

    /// How many instructions the last run executed.
    ///
    /// Exposed because a designer tuning a handler against the fuel limit needs to know how close it
    /// is, and because "it worked on the test map" is not a budget.
    #[must_use]
    pub const fn fuel_used(&self) -> u64 {
        self.fuel_used
    }

    /// Raises an event, running the handler if the program has one.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] when the program does not handle the event, when the argument count
    /// does not match, when a limit is crossed, or when the script itself faults.
    pub fn run(
        &mut self,
        program: &Program,
        host: &mut impl Host,
        event: &str,
        arguments: &[Value],
    ) -> Result<Value, RuntimeError> {
        let Some(&index) = program.events.get(event) else {
            return Err(RuntimeError {
                fault: RuntimeFault::UnhandledEvent {
                    name: event.to_owned(),
                },
                function: event.to_owned(),
                line: 0,
            });
        };
        self.call_function(program, host, index, arguments)
    }

    /// Calls a function by index. `run` is the public door; this is what it goes through.
    fn call_function(
        &mut self,
        program: &Program,
        host: &mut impl Host,
        index: u16,
        arguments: &[Value],
    ) -> Result<Value, RuntimeError> {
        self.stack.clear();
        self.frames.clear();
        self.fuel_used = 0;

        let Some(function) = program.functions.get(index as usize) else {
            return Err(RuntimeError {
                fault: RuntimeFault::Malformed {
                    detail: "event names a function that does not exist",
                },
                function: String::new(),
                line: 0,
            });
        };
        if usize::from(function.arity) != arguments.len() {
            return Err(RuntimeError {
                fault: RuntimeFault::EventArity {
                    expected: function.arity,
                    found: arguments.len(),
                },
                function: function.name.clone(),
                line: function.lines.first().copied().unwrap_or(0),
            });
        }

        self.stack.extend_from_slice(arguments);
        // The arity passed here is what places the frame's base *below* the arguments, so parameter
        // zero is local slot zero. Passing zero instead put the base above them, and every parameter
        // read as the nil that padded the frame.
        self.push_frame(program, index, arguments.len())?;
        self.execute(program, host)
    }

    /// Sets up a frame whose arguments are already the top of the stack.
    fn push_frame(
        &mut self,
        program: &Program,
        index: u16,
        arity: usize,
    ) -> Result<(), RuntimeError> {
        if self.frames.len() >= self.limits.max_call_depth {
            return Err(self.error(
                program,
                RuntimeFault::CallDepth {
                    limit: self.limits.max_call_depth,
                },
            ));
        }
        let Some(function) = program.functions.get(index as usize) else {
            return Err(self.error(
                program,
                RuntimeFault::Malformed {
                    detail: "call names a function that does not exist",
                },
            ));
        };
        let Some(base) = self.stack.len().checked_sub(arity) else {
            return Err(self.error(
                program,
                RuntimeFault::Malformed {
                    detail: "call has fewer arguments on the stack than its arity",
                },
            ));
        };
        // Locals beyond the parameters start as nil, so a `let` that has not run yet reads as nil
        // rather than as whatever the previous frame left in the slot.
        let wanted = base + usize::from(function.locals);
        if wanted > self.limits.max_stack {
            return Err(self.error(
                program,
                RuntimeFault::StackOverflow {
                    limit: self.limits.max_stack,
                },
            ));
        }
        while self.stack.len() < wanted {
            self.stack.push(Value::Nil);
        }
        self.frames.push(Frame {
            function: index,
            ip: 0,
            base,
        });
        Ok(())
    }

    /// Builds an error naming wherever execution currently is.
    fn error(&self, program: &Program, fault: RuntimeFault) -> RuntimeError {
        let (function, line) = self.frames.last().map_or_else(
            || (String::new(), 0),
            |frame| {
                program.functions.get(frame.function as usize).map_or_else(
                    || (String::new(), 0),
                    |function| {
                        let line = function
                            .lines
                            .get(frame.ip.saturating_sub(1))
                            .copied()
                            .unwrap_or(0);
                        (function.name.clone(), line)
                    },
                )
            },
        );
        RuntimeError {
            fault,
            function,
            line,
        }
    }

    fn pop(&mut self, program: &Program) -> Result<Value, RuntimeError> {
        self.stack.pop().ok_or_else(|| {
            self.error(
                program,
                RuntimeFault::Malformed {
                    detail: "an instruction popped an empty stack",
                },
            )
        })
    }

    fn push(&mut self, program: &Program, value: Value) -> Result<(), RuntimeError> {
        if self.stack.len() >= self.limits.max_stack {
            return Err(self.error(
                program,
                RuntimeFault::StackOverflow {
                    limit: self.limits.max_stack,
                },
            ));
        }
        self.stack.push(value);
        Ok(())
    }

    /// Runs until the outermost frame returns.
    #[expect(
        clippy::too_many_lines,
        reason = "an instruction dispatch is one match over the opcode set; splitting it would hide \
                  the one place the whole instruction set can be read at once"
    )]
    fn execute(&mut self, program: &Program, host: &mut impl Host) -> Result<Value, RuntimeError> {
        loop {
            self.fuel_used += 1;
            if self.fuel_used > self.limits.fuel {
                return Err(self.error(
                    program,
                    RuntimeFault::OutOfFuel {
                        limit: self.limits.fuel,
                    },
                ));
            }

            let Some(frame) = self.frames.last().copied() else {
                return Err(self.error(
                    program,
                    RuntimeFault::Malformed {
                        detail: "execution continued with no frame",
                    },
                ));
            };
            let Some(function) = program.functions.get(frame.function as usize) else {
                return Err(self.error(
                    program,
                    RuntimeFault::Malformed {
                        detail: "frame names a function that does not exist",
                    },
                ));
            };
            let Some(&op) = function.code.get(frame.ip) else {
                // Unreachable while every function ends in a `Return`, which the compiler guarantees
                // and a test asserts -- but reported rather than assumed, since the alternative is
                // reading past the end of the code.
                return Err(self.error(
                    program,
                    RuntimeFault::Malformed {
                        detail: "execution ran past the end of a function",
                    },
                ));
            };
            if let Some(last) = self.frames.last_mut() {
                last.ip += 1;
            }

            match op {
                Op::Const(index) => {
                    let Some(&value) = program.constants.get(index as usize) else {
                        return Err(self.error(
                            program,
                            RuntimeFault::Malformed {
                                detail: "constant index is outside the table",
                            },
                        ));
                    };
                    self.push(program, value)?;
                }
                Op::Nil => self.push(program, Value::Nil)?,
                Op::Bool(value) => self.push(program, Value::Bool(value))?,
                Op::GetLocal(slot) => {
                    let Some(&value) = self.stack.get(frame.base + usize::from(slot)) else {
                        return Err(self.error(
                            program,
                            RuntimeFault::Malformed {
                                detail: "local index is outside the frame",
                            },
                        ));
                    };
                    self.push(program, value)?;
                }
                Op::SetLocal(slot) => {
                    let value = self.pop(program)?;
                    let Some(target) = self.stack.get_mut(frame.base + usize::from(slot)) else {
                        return Err(self.error(
                            program,
                            RuntimeFault::Malformed {
                                detail: "local index is outside the frame",
                            },
                        ));
                    };
                    *target = value;
                }
                Op::Arithmetic(operator) => {
                    let right = self.pop(program)?;
                    let left = self.pop(program)?;
                    let value = arithmetic(operator, left, right)
                        .map_err(|error| self.error(program, RuntimeFault::Value(error)))?;
                    self.push(program, value)?;
                }
                Op::Compare(operator) => {
                    let right = self.pop(program)?;
                    let left = self.pop(program)?;
                    let value = compare(operator, left, right)
                        .map_err(|error| self.error(program, RuntimeFault::Value(error)))?;
                    self.push(program, value)?;
                }
                Op::Equal { negated } => {
                    let right = self.pop(program)?;
                    let left = self.pop(program)?;
                    let outcome = equals(left, right) != negated;
                    self.push(program, Value::Bool(outcome))?;
                }
                Op::Negate => {
                    let value = self.pop(program)?;
                    let value = negate(value)
                        .map_err(|error| self.error(program, RuntimeFault::Value(error)))?;
                    self.push(program, value)?;
                }
                Op::Not => {
                    let value = self.pop(program)?;
                    let truth = value
                        .as_bool("!")
                        .map_err(|error| self.error(program, RuntimeFault::Value(error)))?;
                    self.push(program, Value::Bool(!truth))?;
                }
                Op::Pop => {
                    self.pop(program)?;
                }
                Op::Jump(target) => self.jump(target),
                Op::JumpIfFalse(target) => {
                    let value = self.pop(program)?;
                    let truth = value
                        .as_bool("if")
                        .map_err(|error| self.error(program, RuntimeFault::Value(error)))?;
                    if !truth {
                        self.jump(target);
                    }
                }
                Op::JumpIfFalseKeep(target) => {
                    let Some(&value) = self.stack.last() else {
                        return Err(self.error(
                            program,
                            RuntimeFault::Malformed {
                                detail: "a short-circuit jump found an empty stack",
                            },
                        ));
                    };
                    let truth = value
                        .as_bool("&&")
                        .map_err(|error| self.error(program, RuntimeFault::Value(error)))?;
                    if !truth {
                        self.jump(target);
                    }
                }
                Op::JumpIfTrueKeep(target) => {
                    let Some(&value) = self.stack.last() else {
                        return Err(self.error(
                            program,
                            RuntimeFault::Malformed {
                                detail: "a short-circuit jump found an empty stack",
                            },
                        ));
                    };
                    let truth = value
                        .as_bool("||")
                        .map_err(|error| self.error(program, RuntimeFault::Value(error)))?;
                    if truth {
                        self.jump(target);
                    }
                }
                Op::Call(index) => {
                    let Some(callee) = program.functions.get(index as usize) else {
                        return Err(self.error(
                            program,
                            RuntimeFault::Malformed {
                                detail: "call names a function that does not exist",
                            },
                        ));
                    };
                    let arity = usize::from(callee.arity);
                    self.push_frame(program, index, arity)?;
                }
                Op::CallHost(index) => {
                    let Some(signature) = host_arity(program, index) else {
                        return Err(self.error(
                            program,
                            RuntimeFault::Malformed {
                                detail: "host call has no recorded arity",
                            },
                        ));
                    };
                    let Some(start) = self.stack.len().checked_sub(signature) else {
                        return Err(self.error(
                            program,
                            RuntimeFault::Malformed {
                                detail: "host call has fewer arguments on the stack than its arity",
                            },
                        ));
                    };
                    let result = host
                        .call(HostCall {
                            index,
                            arguments: &self.stack[start..],
                            strings: &program.strings,
                        })
                        .map_err(|error| self.error(program, RuntimeFault::Host(error)))?;
                    self.stack.truncate(start);
                    self.push(program, result)?;
                }
                Op::Return => {
                    let value = self.pop(program)?;
                    let Some(frame) = self.frames.pop() else {
                        return Err(self.error(
                            program,
                            RuntimeFault::Malformed {
                                detail: "returned with no frame",
                            },
                        ));
                    };
                    self.stack.truncate(frame.base);
                    if self.frames.is_empty() {
                        return Ok(value);
                    }
                    self.push(program, value)?;
                }
            }
        }
    }

    fn jump(&mut self, target: u16) {
        if let Some(frame) = self.frames.last_mut() {
            frame.ip = target as usize;
        }
    }
}

/// How many arguments a host call at `index` takes.
///
/// Recorded on the program at compile time rather than looked up in an interface at run time, so the
/// machine does not need the interface and a program cannot be run against one that disagrees about
/// arities.
fn host_arity(program: &Program, index: u16) -> Option<usize> {
    program
        .host_arities
        .get(&index)
        .map(|arity| usize::from(*arity))
}

#[cfg(test)]
mod tests {
    use super::{RuntimeFault, RuntimeLimits, Vm};
    use crate::compile::{Program, compile};
    use crate::host::{Host, HostCall, HostError, Interface, StandardHost};
    use crate::parse::Limits;
    use crate::value::Value;

    fn interface() -> Interface {
        let mut interface = Interface::standard();
        interface.declare_event("start", 0).expect("declare");
        interface.declare_event("tick", 1).expect("declare");
        interface
    }

    fn build(source: &str) -> Program {
        compile(source, &interface(), Limits::DEFAULT).expect("compile")
    }

    fn run(source: &str) -> Value {
        let program = build(source);
        let mut host = StandardHost::new();
        Vm::new()
            .run(&program, &mut host, "start", &[])
            .expect("run")
    }

    fn fault(source: &str) -> RuntimeFault {
        let program = build(source);
        let mut host = StandardHost::new();
        Vm::new()
            .run(&program, &mut host, "start", &[])
            .expect_err("fault")
            .fault
    }

    #[test]
    fn arithmetic_and_precedence_produce_the_expected_value() {
        assert_eq!(run("on start() { return 1 + 2 * 3; }"), Value::Int(7));
        assert_eq!(run("on start() { return (1 + 2) * 3; }"), Value::Int(9));
        assert_eq!(run("on start() { return 7 / 2; }"), Value::Int(3));
        assert_eq!(run("on start() { return -5 + 2; }"), Value::Int(-3));
        assert_eq!(run("on start() { return 1 + 0.5; }"), Value::Real(1.5));
    }

    #[test]
    fn a_function_without_a_return_produces_nil() {
        assert_eq!(run("on start() { let x = 1; }"), Value::Nil);
    }

    #[test]
    fn locals_and_assignment_work_and_a_block_local_does_not_escape() {
        assert_eq!(
            run("on start() { let x = 1; x = x + 4; return x; }"),
            Value::Int(5)
        );
        // A local declared in a block leaves scope at its end, so this must not compile -- asserted
        // in the compiler's own tests; here we check the slot reuse does not corrupt an outer local.
        assert_eq!(
            run("on start() { let a = 1; if true { let b = 99; } return a; }"),
            Value::Int(1)
        );
    }

    #[test]
    fn a_local_read_before_its_let_runs_is_nil_rather_than_a_stale_slot() {
        // A frame's locals beyond the parameters start as nil, so a slot reused by a later block
        // cannot leak whatever the previous occupant left there.
        assert_eq!(
            run("on start() { if false { let x = 5; } if true { let y = 1; return y; } }"),
            Value::Int(1)
        );
    }

    #[test]
    fn conditionals_and_loops_run() {
        assert_eq!(
            run("on start() { if 1 < 2 { return 10; } else { return 20; } }"),
            Value::Int(10)
        );
        assert_eq!(
            run("on start() { if 1 > 2 { return 10; } else { return 20; } }"),
            Value::Int(20)
        );
        assert_eq!(
            run(
                "on start() { let total = 0; let i = 0; while i < 5 { total = total + i; i = i + 1; } return total; }"
            ),
            Value::Int(10)
        );
    }

    #[test]
    fn else_if_chains_pick_the_right_branch() {
        let source = "fn grade(n) { if n < 10 { return 1; } else if n < 20 { return 2; } else { return 3; } }
                      on start() { return grade(15); }";
        assert_eq!(run(source), Value::Int(2));
    }

    #[test]
    fn short_circuit_operators_do_not_evaluate_the_far_side() {
        // Asserted through a side effect, because the value alone cannot tell the difference.
        let program = build(
            "fn noisy() { sys.log(\"ran\"); return true; }
             on start() { return false && noisy(); }",
        );
        let mut host = StandardHost::new();
        let value = Vm::new()
            .run(&program, &mut host, "start", &[])
            .expect("run");
        assert_eq!(value, Value::Bool(false));
        assert!(host.log.is_empty(), "the right operand was evaluated");

        let disjunction = build(
            "fn noisy() { sys.log(\"ran\"); return false; }
             on start() { return true || noisy(); }",
        );
        let mut host = StandardHost::new();
        Vm::new()
            .run(&disjunction, &mut host, "start", &[])
            .expect("run");
        assert!(host.log.is_empty());
    }

    #[test]
    fn short_circuit_operators_do_evaluate_the_far_side_when_they_must() {
        let program = build(
            "fn noisy() { sys.log(\"ran\"); return true; }
             on start() { return true && noisy(); }",
        );
        let mut host = StandardHost::new();
        assert_eq!(
            Vm::new()
                .run(&program, &mut host, "start", &[])
                .expect("run"),
            Value::Bool(true)
        );
        assert_eq!(host.log, vec!["ran".to_owned()]);
    }

    #[test]
    fn functions_call_each_other_and_recurse() {
        let source = "fn factorial(n) { if n <= 1 { return 1; } return n * factorial(n - 1); }
                      on start() { return factorial(10); }";
        assert_eq!(run(source), Value::Int(3_628_800));
    }

    #[test]
    fn an_event_receives_its_arguments() {
        let program = build("on tick(elapsed) { return elapsed * 2; }");
        let mut host = StandardHost::new();
        let value = Vm::new()
            .run(&program, &mut host, "tick", &[Value::Int(21)])
            .expect("run");
        assert_eq!(value, Value::Int(42));
    }

    #[test]
    fn raising_an_event_the_script_does_not_handle_is_an_error_rather_than_a_silence() {
        let program = build("on start() {}");
        let mut host = StandardHost::new();
        let error = Vm::new()
            .run(&program, &mut host, "tick", &[Value::Int(1)])
            .expect_err("refuse");
        assert!(matches!(error.fault, RuntimeFault::UnhandledEvent { .. }));

        // A caller that wants silence asks first.
        assert!(!program.handles("tick"));
    }

    #[test]
    fn raising_an_event_with_the_wrong_argument_count_is_refused() {
        let program = build("on tick(elapsed) { return elapsed; }");
        let mut host = StandardHost::new();
        let error = Vm::new()
            .run(&program, &mut host, "tick", &[])
            .expect_err("refuse");
        assert_eq!(
            error.fault,
            RuntimeFault::EventArity {
                expected: 1,
                found: 0
            }
        );
    }

    #[test]
    fn an_infinite_loop_runs_out_of_fuel_rather_than_hanging_the_match() {
        // The property that makes running a mod's script inside a simulation tick defensible.
        let fault = fault("on start() { while true { } }");
        assert_eq!(
            fault,
            RuntimeFault::OutOfFuel {
                limit: RuntimeLimits::DEFAULT.fuel
            }
        );
    }

    #[test]
    fn unbounded_recursion_is_a_call_depth_error_rather_than_a_stack_overflow() {
        // A native stack overflow is an abort with no diagnostic, and this interpreter's frames are
        // heap allocated precisely so the limit can be checked rather than hit.
        let fault = fault("fn forever() { return forever(); } on start() { return forever(); }");
        assert_eq!(
            fault,
            RuntimeFault::CallDepth {
                limit: RuntimeLimits::DEFAULT.max_call_depth
            }
        );
    }

    #[test]
    fn the_fuel_used_is_reported_so_a_handler_can_be_budgeted() {
        let program = build("on start() { let i = 0; while i < 100 { i = i + 1; } }");
        let mut host = StandardHost::new();
        let mut vm = Vm::new();
        vm.run(&program, &mut host, "start", &[]).expect("run");
        let used = vm.fuel_used();
        assert!(used > 100, "a hundred iterations cost more than {used}");
        assert!(used < RuntimeLimits::DEFAULT.fuel);
    }

    #[test]
    fn arithmetic_faults_name_the_line_they_happened_on() {
        let program = build("on start() {\n  let a = 1;\n  return a / 0;\n}");
        let mut host = StandardHost::new();
        let error = Vm::new()
            .run(&program, &mut host, "start", &[])
            .expect_err("fault");
        assert_eq!(error.function, "start");
        assert_eq!(error.line, 3, "{error}");
        assert!(error.to_string().contains("division by zero"), "{error}");
    }

    #[test]
    fn a_non_bool_condition_is_a_fault_rather_than_a_coercion() {
        assert!(matches!(
            fault("on start() { if 1 { return 2; } }"),
            RuntimeFault::Value(_)
        ));
        assert!(matches!(
            fault("on start() { while nil { } }"),
            RuntimeFault::Value(_)
        ));
    }

    #[test]
    fn overflow_in_a_script_is_a_fault_rather_than_a_panic() {
        // A script must not be able to take the process down; a panic mid-tick takes the match too.
        assert!(matches!(
            fault("fn f(n) { return n * n; } on start() { return f(f(f(999999))); }"),
            RuntimeFault::Value(_)
        ));

        // And a real leaving the range is a fault rather than an infinity, because an infinity
        // becomes a NaN one subtraction later and every comparison against a NaN is false.
        assert!(matches!(
            fault("fn f(n) { return n * n; } on start() { return f(f(f(1.5e100))); }"),
            RuntimeFault::Value(_)
        ));
    }

    #[test]
    fn the_standard_functions_are_reachable_and_deterministic() {
        assert_eq!(run("on start() { return sys.sqrt(16); }"), Value::Real(4.0));
        assert_eq!(run("on start() { return sys.floor(2.75); }"), Value::Int(2));
        assert_eq!(
            run("on start() { return sys.clamp(99, 0, 10); }"),
            Value::Real(10.0)
        );

        // Angles are turns, so half a revolution is half a turn.
        let Value::Real(half) = run("on start() { return sys.cos(0.5); }") else {
            panic!("expected a real");
        };
        assert!(
            (half + 1.0).abs() < 1e-15,
            "cosine of half a turn was {half}"
        );
    }

    #[test]
    fn a_host_that_refuses_produces_a_fault_naming_the_line() {
        struct Refusing;
        impl Host for Refusing {
            fn call(&mut self, _call: HostCall<'_>) -> Result<Value, HostError> {
                Err(HostError::Message {
                    detail: "not allowed here".to_owned(),
                })
            }
        }
        let program = build("on start() {\n  sys.log(\"x\");\n}");
        let error = Vm::new()
            .run(&program, &mut Refusing, "start", &[])
            .expect_err("fault");
        assert_eq!(error.line, 2);
        assert!(error.to_string().contains("not allowed here"), "{error}");
    }

    #[test]
    fn strings_compare_by_value_through_their_interned_handles() {
        assert_eq!(
            run("on start() { return \"alpha\" == \"alpha\"; }"),
            Value::Bool(true)
        );
        assert_eq!(
            run("on start() { return \"alpha\" == \"beta\"; }"),
            Value::Bool(false)
        );
    }

    #[test]
    fn the_machine_keeps_nothing_between_runs() {
        // A script with hidden globals would be simulation state the desync report cannot see.
        let program = build("on start() { let x = 1; return x; }");
        let mut host = StandardHost::new();
        let mut vm = Vm::new();
        for _ in 0..8 {
            assert_eq!(
                vm.run(&program, &mut host, "start", &[]).expect("run"),
                Value::Int(1)
            );
            assert!(vm.stack.is_empty());
            assert!(vm.frames.is_empty());
        }
    }

    #[test]
    fn a_deep_expression_hits_the_stack_limit_rather_than_growing_without_bound() {
        let program =
            build("fn f(a, b, c, d) { return a + b + c + d; } on start() { return f(1,2,3,4); }");
        let mut host = StandardHost::new();
        let mut vm = Vm::with_limits(RuntimeLimits {
            max_stack: 2,
            ..RuntimeLimits::DEFAULT
        });
        let error = vm
            .run(&program, &mut host, "start", &[])
            .expect_err("refuse");
        assert!(matches!(error.fault, RuntimeFault::StackOverflow { .. }));
    }

    #[test]
    fn two_runs_of_the_same_script_produce_the_same_answer() {
        // The claim the whole crate exists to support. ADR 0007's restricted operation set is what
        // makes it true across machines; this pins that nothing stateful crept into the interpreter.
        let source = "fn walk(n) { let total = 0.0; let i = 0;
                        while i < n { total = total + sys.sin(i * 0.017) * 3.25; i = i + 1; }
                        return total; }
                      on start() { return walk(200); }";
        assert_eq!(run(source), run(source));
    }

    #[test]
    fn a_computed_result_is_pinned_to_exact_bits() {
        // An approximate assertion about a value whose whole purpose is bit-exactness verifies
        // nothing -- ADR 0007 decision 4, applied to the interpreter rather than to one function.
        // Changing the evaluation order, the promotion rules, or the sine polynomial fails here
        // rather than silently desyncing a match.
        let source = "fn distance(ax, ay, bx, by) {
                        let dx = bx - ax; let dy = by - ay;
                        return sys.sqrt(dx * dx + dy * dy); }
                      on start() { return distance(0.5, 1.25, 12.75, 9.5); }";
        let Value::Real(value) = run(source) else {
            panic!("expected a real");
        };
        assert_eq!(
            value.to_bits(),
            0x402D_89C1_A411_5264,
            "distance moved: {value:?} is {:#018X}",
            value.to_bits()
        );
    }
}
