//! The dispatcher: which scripts hear which events, in what order, and what a mission remembers.
//!
//! [ADR 7002](../../../docs/adr/7002-script-events.md) is the design and this is the implementation.
//! Four of its decisions shape everything below, so they are worth restating where the code is:
//!
//! - **A handler is the subscription.** There is no registration call, no `main`, and no subscribe
//!   verb. A script subscribes to `tick` by declaring `on tick(elapsed)`, [`Program::handles`]
//!   reports that, and the dispatcher raises an event only to the scripts that say yes. The VM's rule
//!   that raising an unhandled event is an *error* is left exactly as it is: it guards against
//!   raising blind, and the `handles` check is what makes it unreachable here.
//! - **Authored order.** Scripts run in the order the scenario listed them, back to back, within one
//!   tick. Determinism needs *an* order, and this is the one a designer can see in a diff and change.
//! - **What a script remembers is kernel state.** Flags, counters and timers live in [`Mission`], on
//!   this side of the host boundary, hashed every tick and replayed with everything else. A script
//!   runtime holding globals of its own would be simulation state the desync report cannot see.
//! - **A fault disables that script for the rest of the run.** The fault is deterministic — same
//!   tick, same instruction, every machine — so the disabling is too. Killing the match instead would
//!   hand a mod author a denial of service on every player; re-raising into a script that just proved
//!   itself wrong turns one diagnostic into one per tick.
//!
//! # Why this declares three events where the ADR designs five
//!
//! ADR 7002 decision 4 specifies `start`, `tick`, `timer_elapsed`, `zone_entered` and `zone_exited`.
//! The zone pair needs zones in the scenario format and units to test against them, so it arrives with
//! the M6 capabilities that supply it.
//!
//! Declaring them ahead of that would cost exactly what the closed event set is for: `on
//! zone_entered(zone, unit)` would compile and then never fire, which is indistinguishable from a
//! handler whose body is wrong. Undeclared, it is a compile error naming the line and listing the
//! events that do exist. An event added later is backwards-compatible — an old script simply does not
//! handle it — so there is no cost to waiting and a real one to promising early.
//!
//! # A `str` argument is an index into the receiving script's own constants
//!
//! A string in this language is an index into the program's constant table; there is no heap, so
//! there is no way to synthesize one at run time. `timer_elapsed(timer)` therefore resolves the
//! timer's name against each receiving program's table, and a script whose source never mentions that
//! name does not hear about it.
//!
//! That falls out of the value model rather than being a policy, and the alternative is worse: a
//! handler handed a value it cannot compare against any of its own literals cannot act on it, so
//! raising would spend fuel to accomplish nothing. It is also visible — every name a script can react
//! to is written in the script.
//!
//! # Where to register it
//!
//! Registration order is execution order, and this subsystem is the one that will eventually issue
//! orders, so it belongs *before* the subsystems that carry them out. That keeps "the tick a script
//! decides" and "the tick the decision takes effect" the same tick, which is the rule
//! [`units`](crate::units) already follows for a player's commands.

use std::collections::{BTreeMap, VecDeque};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_script::host::STANDARD;
use cic_script::value::type_name;
use cic_script::{
    CompileError, Host, HostCall, HostError, Interface, Limits, Program, RuntimeLimits,
    StandardHost, Value, Vm, compile,
};

use crate::hash::StateHasher;
use crate::subsystem::{Subsystem, TickContext};

/// The name the [`Scripts`] subsystem is registered and hashed under.
pub const SCRIPTS: &str = "scripts";

/// Raised once, before the first [`EVENT_TICK`], after activation has placed everything.
pub const EVENT_START: &str = "start";

/// Raised every simulation tick, with the fixed tick length in seconds.
pub const EVENT_TICK: &str = "tick";

/// Raised when a timer armed through `sys.arm_timer` runs out, with the timer's name.
pub const EVENT_TIMER_ELAPSED: &str = "timer_elapsed";

/// The events this kernel raises, with their arities.
///
/// A closed, versioned set: renaming or removing one is a break that takes a new interface version,
/// while adding one is backwards-compatible.
pub const EVENTS: [(&str, u8); 3] = [(EVENT_START, 0), (EVENT_TICK, 1), (EVENT_TIMER_ELAPSED, 1)];

/// The mission verbs, in the order their indices are assigned after [`STANDARD`].
///
/// One table, read by both [`interface`] and [`Mission::call`], so a verb cannot be declared at one
/// index and implemented at another. A test asserts the two agree.
pub const VERBS: [(&str, u8); 7] = [
    ("flag", 1),
    ("set_flag", 2),
    ("counter", 1),
    ("add_counter", 2),
    ("arm_timer", 2),
    ("cancel_timer", 1),
    ("timer_pending", 1),
];

/// How many `sys.log` lines are retained for diagnostics.
///
/// The *count* is unbounded and hashed; the retained text is neither. A thirty-minute match at thirty
/// ticks a second is fifty-four thousand ticks, and a handler logging on every one of them would
/// otherwise grow a `Vec` inside the simulation for the whole match.
const MESSAGES_RETAINED: usize = 256;

/// How many faults are retained. A script is disabled by its first one, so this bounds at one per
/// script plus the handful a multi-script scenario can produce in the same tick.
const FAULTS_RETAINED: usize = 64;

/// 2^64, the first `f64` too large to be a `u64`.
///
/// Written out rather than computed, because ADR 0007 forbids `powi` — not for inexactness but
/// because its lowering is unspecified. 2^64 is exactly representable, so the literal is the same
/// number with no instruction to argue about.
const TICK_LIMIT: f64 = 18_446_744_073_709_551_616.0;

/// The interface a scenario's scripts compile against.
///
/// The standard functions, the mission verbs, and the events this kernel raises — and nothing else,
/// which is the security model rather than a convenience. This is the engine's compatibility surface:
/// what a `.cicmap`'s scripts are compiled against, so its names and arities are versioned and its
/// changes are release notes.
///
/// # Panics
///
/// Panics if [`VERBS`] repeats a name or collides with [`STANDARD`], or if [`EVENTS`] repeats one.
/// Each is a fault in this file rather than in a caller, and a test covers all three.
#[must_use]
pub fn interface() -> Interface {
    let mut interface = Interface::standard();
    for (name, arity) in VERBS {
        interface
            .declare_function(name, arity)
            .expect("the verb table has no name the standard set already took");
    }
    for (name, arity) in EVENTS {
        interface
            .declare_event(name, arity)
            .expect("the event table has no repeated names");
    }
    interface
}

/// One compiled script, and where it came from.
#[derive(Debug, Clone)]
struct Script {
    /// The package-relative path, for a diagnostic that names the file.
    path: String,
    program: Program,
}

/// A handler that faulted, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptFault {
    /// The tick it happened on.
    pub tick: u64,
    /// The script's package-relative path.
    pub path: String,
    /// The event whose handler faulted.
    pub event: String,
    /// The diagnostic, which names the function and the line inside the script.
    pub detail: String,
}

impl Display for ScriptFault {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "tick {}: `{}` handling `{}`: {}",
            self.tick, self.path, self.event, self.detail
        )
    }
}

/// A script that failed to compile, and which one.
#[derive(Debug)]
pub struct ScriptLoadError {
    /// The package-relative path of the script that failed.
    pub path: String,
    /// What the compiler said, including the line.
    pub error: CompileError,
}

impl Display for ScriptLoadError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path, self.error)
    }
}

impl Error for ScriptLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.error)
    }
}

/// What the scripts of one match remember, on the kernel's side of the host boundary.
///
/// Every field here is hashed, which is the point of the type existing: ADR 7002 decision 7 puts
/// mission memory on this side precisely so it is recorded, replayed, and visible to desync
/// diagnosis. A script cannot hold it, because the language has no globals.
#[derive(Debug, Clone, Default)]
pub struct Mission {
    /// The standard set's implementation, delegated to rather than reimplemented.
    standard: StandardHost,
    flags: BTreeMap<String, bool>,
    counters: BTreeMap<String, i64>,
    /// Armed timers: the name a script armed, against the tick it fires on.
    timers: BTreeMap<String, u64>,
    /// The tick being dispatched. Timers arm relative to it, so the dispatcher sets it before
    /// raising anything.
    tick: u64,
    /// The fixed tick length, for converting an armed duration into a deadline.
    tick_seconds: f64,
    /// Every line `sys.log` has produced, counted. Hashed, for the reason
    /// [`Units::rejected`](crate::units::Units::rejected) is: a machine that took a different path
    /// through a handler without changing a flag would otherwise drift silently.
    messages_logged: u64,
    messages: VecDeque<String>,
    /// Verbs refused: a duration that is not a finite non-negative number, a counter that would
    /// overflow its name. Counted and hashed for the same reason.
    refused: u64,
}

impl Mission {
    /// Whether a flag is set. An unset flag reads as `false` rather than being an error, so a script
    /// can test a flag it has not written.
    #[must_use]
    pub fn flag(&self, name: &str) -> bool {
        self.flags.get(name).copied().unwrap_or(false)
    }

    /// A counter's value. An unwritten counter reads as zero.
    #[must_use]
    pub fn counter(&self, name: &str) -> i64 {
        self.counters.get(name).copied().unwrap_or(0)
    }

    /// The armed timers, against the tick each fires on.
    #[must_use]
    pub fn timers(&self) -> &BTreeMap<String, u64> {
        &self.timers
    }

    /// The retained `sys.log` lines, oldest first.
    #[must_use]
    pub fn messages(&self) -> Vec<&str> {
        self.messages.iter().map(String::as_str).collect()
    }

    /// How many lines `sys.log` has produced in total, including any no longer retained.
    #[must_use]
    pub fn messages_logged(&self) -> u64 {
        self.messages_logged
    }

    /// How many verb calls were refused.
    #[must_use]
    pub fn refused(&self) -> u64 {
        self.refused
    }

    /// The tick an armed duration falls due on, or `None` for a duration that cannot be one.
    fn deadline(&self, seconds: f64) -> Option<u64> {
        if !(seconds.is_finite() && seconds >= 0.0) {
            return None;
        }
        if !(self.tick_seconds.is_finite() && self.tick_seconds > 0.0) {
            return None;
        }
        // Division and rounding are both on ADR 0007's permitted list, so every machine converts the
        // same duration into the same number of ticks.
        let ticks = (seconds / self.tick_seconds).ceil();
        if !(0.0..TICK_LIMIT).contains(&ticks) {
            return None;
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "range checked immediately above, and `ceil` left no fraction to truncate"
        )]
        let ticks = ticks as u64;
        // A timer never fires in the tick that armed it: a zero-second timer would otherwise re-enter
        // the dispatch pass it was armed from, and a handler re-arming itself would loop within one
        // tick until it ran out of fuel.
        self.tick.checked_add(ticks.max(1))
    }

    /// Folds the mission's whole state into the tick hash.
    fn write_state(&self, hasher: &mut StateHasher) {
        hasher.write_u64(self.flags.len() as u64);
        for (name, &value) in &self.flags {
            hasher.write_str(name);
            hasher.write_bytes(&[u8::from(value)]);
        }
        hasher.write_u64(self.counters.len() as u64);
        for (name, &value) in &self.counters {
            hasher.write_str(name);
            hasher.write_i64(value);
        }
        hasher.write_u64(self.timers.len() as u64);
        for (name, &deadline) in &self.timers {
            hasher.write_str(name);
            hasher.write_u64(deadline);
        }
        hasher.write_u64(self.messages_logged);
        hasher.write_u64(self.refused);
    }
}

/// Reads an argument as a truth value.
///
/// There is no coercion: a `bool` parameter takes a `bool`. Accepting a number here would be the
/// truthiness the language deliberately does not have, one layer down.
fn boolean(
    call: &HostCall<'_>,
    position: usize,
    function: &'static str,
) -> Result<bool, HostError> {
    match call.arguments.get(position) {
        Some(Value::Bool(value)) => Ok(*value),
        other => Err(HostError::Type {
            function,
            position,
            expected: "a bool",
            found: other.map_or("nothing", |value| type_name(*value)),
        }),
    }
}

/// Reads an argument as a whole number.
fn integer(call: &HostCall<'_>, position: usize, function: &'static str) -> Result<i64, HostError> {
    match call.arguments.get(position) {
        Some(Value::Int(value)) => Ok(*value),
        other => Err(HostError::Type {
            function,
            position,
            expected: "an int",
            found: other.map_or("nothing", |value| type_name(*value)),
        }),
    }
}

impl Host for Mission {
    fn call(&mut self, call: HostCall<'_>) -> Result<Value, HostError> {
        let Some(offset) = (call.index as usize).checked_sub(STANDARD.len()) else {
            let value = self.standard.call(call)?;
            // Only `sys.log` pushes, so draining here bounds the retained text without
            // reimplementing the standard set's dispatch and letting the two tables drift.
            for line in self.standard.log.drain(..) {
                self.messages_logged += 1;
                if self.messages.len() == MESSAGES_RETAINED {
                    self.messages.pop_front();
                }
                self.messages.push_back(line);
            }
            return Ok(value);
        };

        let Some(&(name, _)) = VERBS.get(offset) else {
            return Err(HostError::Unimplemented { index: call.index });
        };

        let value = match name {
            "flag" => Value::Bool(self.flag(call.text(0, "flag")?)),
            "set_flag" => {
                let set = boolean(&call, 1, "set_flag")?;
                let name = call.text(0, "set_flag")?;
                self.flags.insert(name.to_owned(), set);
                Value::Nil
            }
            "counter" => Value::Int(self.counter(call.text(0, "counter")?)),
            "add_counter" => {
                let delta = integer(&call, 1, "add_counter")?;
                let name = call.text(0, "add_counter")?;
                // Overflow is reported rather than saturated: a counter that silently stops
                // counting is a mission rule that silently stops firing.
                let Some(total) = self.counter(name).checked_add(delta) else {
                    self.refused += 1;
                    return Err(HostError::Message {
                        detail: format!("counter `{name}` would overflow"),
                    });
                };
                self.counters.insert(name.to_owned(), total);
                Value::Int(total)
            }
            "arm_timer" => {
                let seconds = call.number(1, "arm_timer")?;
                let name = call.text(0, "arm_timer")?;
                let Some(deadline) = self.deadline(seconds) else {
                    // Refused rather than faulted: a duration computed from mission arithmetic can
                    // legitimately come out nonsensical, and taking the script out of the run for it
                    // is a heavier response than the mistake deserves. The count is hashed, so the
                    // refusal is visible rather than silent.
                    self.refused += 1;
                    return Ok(Value::Bool(false));
                };
                // Re-arming a live timer replaces it, so a script does not have to cancel first.
                self.timers.insert(name.to_owned(), deadline);
                Value::Bool(true)
            }
            "cancel_timer" => {
                let name = call.text(0, "cancel_timer")?;
                Value::Bool(self.timers.remove(name).is_some())
            }
            "timer_pending" => {
                let name = call.text(0, "timer_pending")?;
                Value::Bool(self.timers.contains_key(name))
            }
            _ => return Err(HostError::Unimplemented { index: call.index }),
        };
        Ok(value)
    }
}

/// How one event's arguments are built for each receiving program.
enum Arguments<'a> {
    /// The same values for every script.
    Fixed(&'a [Value]),
    /// One string, resolved against each program's own constant table.
    Text(&'a str),
}

/// A scenario's scripts, and the events they receive.
///
/// Construct with [`Scripts::compile`], register with
/// [`Kernel::add_subsystem`](crate::kernel::Kernel::add_subsystem).
#[derive(Debug, Clone)]
pub struct Scripts {
    /// Compiled scripts in authored order, which is dispatch order.
    compiled: Vec<Script>,
    /// Parallel to `compiled`: whether a fault has taken one out of the run.
    disabled: Vec<bool>,
    mission: Mission,
    limits: RuntimeLimits,
    /// Whether [`EVENT_START`] has been raised.
    started: bool,
    faults: VecDeque<ScriptFault>,
    faulted: u64,
    /// The most fuel any single handler has used, for a designer tuning against the limit.
    peak_fuel: u64,
}

impl Scripts {
    /// Compiles a scenario's scripts, in authored order, against [`interface`].
    ///
    /// Every script is compiled at load, which is where a script naming a verb or an event the engine
    /// does not offer fails — with the file, the line, and what *was* available. Nothing is scanned
    /// for or discovered: a script the scenario does not list does not run, however it got into the
    /// archive.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptLoadError`] for the first script that fails to compile, naming its path.
    pub fn compile(
        sources: &[(&str, &str)],
        limits: Limits,
        runtime: RuntimeLimits,
    ) -> Result<Self, ScriptLoadError> {
        let interface = interface();
        let mut compiled = Vec::with_capacity(sources.len());
        for (path, source) in sources {
            let program = compile(source, &interface, limits).map_err(|error| ScriptLoadError {
                path: (*path).to_owned(),
                error,
            })?;
            compiled.push(Script {
                path: (*path).to_owned(),
                program,
            });
        }
        Ok(Self {
            disabled: vec![false; compiled.len()],
            compiled,
            mission: Mission::default(),
            limits: runtime,
            started: false,
            faults: VecDeque::new(),
            faulted: 0,
            peak_fuel: 0,
        })
    }

    /// What the scripts remember.
    #[must_use]
    pub fn mission(&self) -> &Mission {
        &self.mission
    }

    /// The scripts' paths, in dispatch order.
    #[must_use]
    pub fn paths(&self) -> Vec<&str> {
        self.compiled
            .iter()
            .map(|script| script.path.as_str())
            .collect()
    }

    /// Whether the script at an index has been disabled by a fault.
    #[must_use]
    pub fn is_disabled(&self, index: usize) -> bool {
        self.disabled.get(index).copied().unwrap_or(false)
    }

    /// The retained faults, oldest first.
    #[must_use]
    pub fn faults(&self) -> Vec<&ScriptFault> {
        self.faults.iter().collect()
    }

    /// How many handlers have faulted in total.
    #[must_use]
    pub fn faulted(&self) -> u64 {
        self.faulted
    }

    /// The most fuel any one handler has used.
    ///
    /// Deliberately *not* hashed. Fuel is the interpreter's accounting rather than the mission's
    /// state, so pinning it would make every recorded replay depend on how many instructions this
    /// crate's compiler happens to emit.
    #[must_use]
    pub fn peak_fuel(&self) -> u64 {
        self.peak_fuel
    }

    /// Raises one event to every script that handles it, in authored order.
    fn raise(&mut self, event: &str, arguments: &Arguments<'_>, tick: u64) {
        for index in 0..self.compiled.len() {
            if self.disabled[index] {
                continue;
            }
            let script = &self.compiled[index];
            if !script.program.handles(event) {
                continue;
            }

            // One machine per handler, so nothing leaks from one script's run into the next. `Vm`
            // holds two empty `Vec`s until a run pushes, so this costs nothing until it is used.
            let mut vm = Vm::with_limits(self.limits);
            let outcome = match arguments {
                Arguments::Fixed(values) => {
                    vm.run(&script.program, &mut self.mission, event, values)
                }
                Arguments::Text(text) => {
                    let Some(value) = intern(&script.program, text) else {
                        continue;
                    };
                    vm.run(&script.program, &mut self.mission, event, &[value])
                }
            };
            self.peak_fuel = self.peak_fuel.max(vm.fuel_used());

            if let Err(error) = outcome {
                self.faulted += 1;
                if self.faults.len() == FAULTS_RETAINED {
                    self.faults.pop_front();
                }
                self.faults.push_back(ScriptFault {
                    tick,
                    path: script.path.clone(),
                    event: event.to_owned(),
                    detail: error.to_string(),
                });
                self.disabled[index] = true;
            }
        }
    }
}

/// Resolves text to an identical constant in the program's own table.
///
/// `None` means the script holds no such string, so it could not compare the argument against
/// anything it contains — see the module documentation for why not raising beats raising a value the
/// handler cannot name.
fn intern(program: &Program, text: &str) -> Option<Value> {
    let index = program
        .strings()
        .iter()
        .position(|constant| constant == text)?;
    u16::try_from(index).ok().map(Value::Str)
}

impl Subsystem for Scripts {
    fn name(&self) -> &'static str {
        SCRIPTS
    }

    fn tick(&mut self, context: &mut TickContext<'_>) {
        self.mission.tick = context.tick;
        self.mission.tick_seconds = context.tick_seconds;

        if !self.started {
            self.started = true;
            self.raise(EVENT_START, &Arguments::Fixed(&[]), context.tick);
        }

        self.raise(
            EVENT_TICK,
            &Arguments::Fixed(&[Value::Real(context.tick_seconds)]),
            context.tick,
        );

        // Timers fall due after the tick handlers, so a handler arming one and a handler reacting to
        // one are never in the same pass. Due timers are collected first and in name order, which is
        // what makes the sequence identical on every machine; each is removed before its handler
        // runs, so a handler may re-arm its own name without the new timer being consumed here.
        let due: Vec<String> = self
            .mission
            .timers
            .iter()
            .filter(|&(_, &deadline)| deadline <= context.tick)
            .map(|(name, _)| name.clone())
            .collect();
        for name in due {
            self.mission.timers.remove(&name);
            self.raise(EVENT_TIMER_ELAPSED, &Arguments::Text(&name), context.tick);
        }
    }

    fn write_state(&self, hasher: &mut StateHasher) {
        hasher.write_bytes(&[u8::from(self.started)]);
        hasher.write_u64(self.disabled.len() as u64);
        for &disabled in &self.disabled {
            hasher.write_bytes(&[u8::from(disabled)]);
        }
        hasher.write_u64(self.faulted);
        self.mission.write_state(hasher);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use cic_script::{Limits, RuntimeLimits};

    use super::{EVENTS, SCRIPTS, Scripts, VERBS, interface};
    use crate::kernel::{Kernel, KernelConfig, first_divergence};

    /// Thirty ticks a second, so a second is thirty ticks and the arithmetic in a test is obvious.
    fn kernel(sources: &[(&str, &str)]) -> Kernel {
        let scripts = Scripts::compile(sources, Limits::DEFAULT, RuntimeLimits::DEFAULT)
            .expect("the test scripts compile");
        let mut kernel = Kernel::new(KernelConfig {
            seed: 7,
            ticks_per_second: 30,
        });
        kernel.add_subsystem(Box::new(scripts));
        kernel
    }

    fn scripts(kernel: &Kernel) -> &Scripts {
        kernel
            .subsystem(SCRIPTS)
            .and_then(|subsystem| subsystem.as_any().downcast_ref::<Scripts>())
            .expect("scripts registered")
    }

    #[test]
    fn the_verb_and_event_tables_are_the_only_source_of_indices() {
        // Declared at one index and implemented at another is exactly the bug two tables allow.
        let interface = interface();
        for (offset, (name, arity)) in VERBS.iter().enumerate() {
            let (index, declared) = interface.function(name).expect("declared");
            assert_eq!(
                usize::from(index),
                cic_script::host::STANDARD.len() + offset,
                "`{name}` moved"
            );
            assert_eq!(declared, *arity);
        }
        for (position, (name, arity)) in EVENTS.iter().enumerate() {
            let (index, declared) = interface.event(name).expect("declared");
            assert_eq!(usize::from(index), position, "`{name}` moved");
            assert_eq!(declared, *arity);
        }
    }

    #[test]
    fn a_handler_is_the_subscription() {
        // No registration call: declaring `on start` is what subscribes, and a script with no `tick`
        // handler is skipped rather than being an error.
        let mut kernel = kernel(&[(
            "scripts/mission.cics",
            r#"on start() { sys.set_flag("started", true); }"#,
        )]);
        kernel.advance(&[]).expect("advances");
        assert!(scripts(&kernel).mission().flag("started"));
        assert_eq!(scripts(&kernel).faulted(), 0);
    }

    #[test]
    fn start_is_raised_once_and_before_the_first_tick() {
        let mut kernel = kernel(&[(
            "scripts/mission.cics",
            r#"
            on start() { sys.add_counter("starts", 1); }
            on tick(elapsed) {
                sys.add_counter("ticks", 1);
                if sys.counter("starts") == 0 { sys.set_flag("tick_ran_first", true); }
            }
            "#,
        )]);
        for _ in 0..5 {
            kernel.advance(&[]).expect("advances");
        }
        let mission = scripts(&kernel).mission();
        assert_eq!(mission.counter("starts"), 1, "start is raised once");
        assert_eq!(mission.counter("ticks"), 5);
        assert!(!mission.flag("tick_ran_first"), "start precedes the tick");
    }

    #[test]
    fn an_event_the_kernel_does_not_declare_fails_to_compile() {
        // The zone pair is designed in ADR 7002 and not declared until it can be raised, so a script
        // handling one is a compile error listing what does exist -- rather than a handler that
        // silently never fires.
        let error = Scripts::compile(
            &[("scripts/zones.cics", "on zone_entered(zone, unit) { }")],
            Limits::DEFAULT,
            RuntimeLimits::DEFAULT,
        )
        .expect_err("an undeclared event must not compile");
        assert_eq!(error.path, "scripts/zones.cics");
        let message = error.to_string();
        assert!(message.contains("zone_entered"), "{message}");
        assert!(message.contains("tick"), "it lists what exists: {message}");
    }

    #[test]
    fn a_verb_the_kernel_does_not_offer_fails_to_compile() {
        let error = Scripts::compile(
            &[(
                "scripts/greedy.cics",
                "on start() { sys.grant_resources(9); }",
            )],
            Limits::DEFAULT,
            RuntimeLimits::DEFAULT,
        )
        .expect_err("an undeclared verb must not compile");
        assert!(error.to_string().contains("grant_resources"));
    }

    #[test]
    fn scripts_dispatch_in_authored_order() {
        // The second script can see what the first did this tick, and not the other way round.
        let sources = [
            (
                "scripts/first.cics",
                r#"on tick(e) { sys.add_counter("n", 1); }"#,
            ),
            (
                "scripts/second.cics",
                r#"on tick(e) { if sys.counter("n") == 1 { sys.set_flag("saw_first", true); } }"#,
            ),
        ];
        let ran = |order: &[(&str, &str)]| {
            let mut host = kernel(order);
            host.advance(&[]).expect("advances");
            scripts(&host).mission().flag("saw_first")
        };
        assert!(ran(&sources));
        assert!(
            !ran(&[sources[1], sources[0]]),
            "reversing the authored order reverses the dispatch order"
        );
    }

    #[test]
    fn a_timer_fires_on_its_due_tick_and_only_once() {
        let mut kernel = kernel(&[(
            "scripts/mission.cics",
            r#"
            on start() { sys.arm_timer("wave", 1.0); }
            on timer_elapsed(timer) { sys.add_counter("fired", 1); }
            "#,
        )]);
        // One second at thirty ticks a second: armed on tick zero, due on tick thirty.
        for _ in 0..30 {
            kernel.advance(&[]).expect("advances");
        }
        assert_eq!(
            scripts(&kernel).mission().counter("fired"),
            0,
            "it must not fire before its deadline"
        );
        kernel.advance(&[]).expect("advances");
        assert_eq!(scripts(&kernel).mission().counter("fired"), 1);
        for _ in 0..10 {
            kernel.advance(&[]).expect("advances");
        }
        assert_eq!(
            scripts(&kernel).mission().counter("fired"),
            1,
            "an elapsed timer is consumed"
        );
        assert!(scripts(&kernel).mission().timers().is_empty());
    }

    #[test]
    fn a_zero_second_timer_fires_on_the_next_tick_not_the_arming_one() {
        // Otherwise a handler re-arming itself loops inside one tick until it runs out of fuel.
        let mut kernel = kernel(&[(
            "scripts/mission.cics",
            r#"
            on start() { sys.arm_timer("now", 0); }
            on timer_elapsed(timer) { sys.add_counter("fired", 1); }
            "#,
        )]);
        kernel.advance(&[]).expect("advances");
        assert_eq!(scripts(&kernel).mission().counter("fired"), 0);
        kernel.advance(&[]).expect("advances");
        assert_eq!(scripts(&kernel).mission().counter("fired"), 1);
    }

    #[test]
    fn a_re_armed_timer_is_not_consumed_by_the_pass_that_delivered_it() {
        let mut kernel = kernel(&[(
            "scripts/mission.cics",
            r#"
            on start() { sys.arm_timer("beat", 0); }
            on timer_elapsed(timer) {
                sys.add_counter("beats", 1);
                sys.arm_timer("beat", 0);
            }
            "#,
        )]);
        for _ in 0..6 {
            kernel.advance(&[]).expect("advances");
        }
        // Armed on tick 0, so it beats on ticks 1 through 5: five beats, one per tick, and no
        // runaway within a single tick.
        assert_eq!(scripts(&kernel).mission().counter("beats"), 5);
    }

    #[test]
    fn a_timer_reaches_only_the_scripts_that_name_it() {
        // A string is an index into the receiving program's constants, so a script that never
        // mentions the name has nothing to compare the argument against.
        let mut kernel = kernel(&[
            (
                "scripts/armer.cics",
                r#"on start() { sys.arm_timer("wave", 0); }"#,
            ),
            (
                "scripts/named.cics",
                r#"on timer_elapsed(timer) { if timer == "wave" { sys.set_flag("named", true); } }"#,
            ),
            (
                "scripts/unnamed.cics",
                r#"on timer_elapsed(timer) { sys.set_flag("unnamed", true); }"#,
            ),
        ]);
        kernel.advance(&[]).expect("advances");
        kernel.advance(&[]).expect("advances");
        let mission = scripts(&kernel).mission();
        assert!(
            mission.flag("named"),
            "the script naming the timer hears it"
        );
        assert!(
            !mission.flag("unnamed"),
            "a script that cannot name the timer is not raised to"
        );
    }

    #[test]
    fn cancelling_a_timer_stops_it_and_reports_whether_it_was_armed() {
        let mut kernel = kernel(&[(
            "scripts/mission.cics",
            r#"
            on start() {
                sys.arm_timer("wave", 0);
                if sys.timer_pending("wave") { sys.set_flag("pending", true); }
                if sys.cancel_timer("wave") { sys.set_flag("cancelled", true); }
                if sys.cancel_timer("wave") { sys.set_flag("cancelled_twice", true); }
            }
            on timer_elapsed(timer) { sys.set_flag("fired", true); }
            "#,
        )]);
        kernel.advance(&[]).expect("advances");
        kernel.advance(&[]).expect("advances");
        let mission = scripts(&kernel).mission();
        assert!(mission.flag("pending"));
        assert!(mission.flag("cancelled"));
        assert!(!mission.flag("cancelled_twice"));
        assert!(!mission.flag("fired"), "a cancelled timer does not fire");
    }

    #[test]
    fn a_nonsense_duration_is_refused_and_counted_rather_than_faulting() {
        let mut kernel = kernel(&[(
            "scripts/mission.cics",
            r#"
            on start() {
                if sys.arm_timer("bad", 0 - 5) { sys.set_flag("armed", true); }
                sys.set_flag("still_running", true);
            }
            "#,
        )]);
        kernel.advance(&[]).expect("advances");
        let subsystem = scripts(&kernel);
        assert!(!subsystem.mission().flag("armed"));
        assert!(
            subsystem.mission().flag("still_running"),
            "a refused verb reports and continues"
        );
        assert_eq!(subsystem.mission().refused(), 1);
        assert_eq!(subsystem.faulted(), 0);
    }

    #[test]
    fn a_fault_disables_that_script_and_leaves_the_others_running() {
        let mut kernel = kernel(&[
            (
                "scripts/runaway.cics",
                r"on tick(e) { let n = 0; while true { n = n + 1; } }",
            ),
            (
                "scripts/sound.cics",
                r#"on tick(e) { sys.add_counter("ticks", 1); }"#,
            ),
        ]);
        for _ in 0..4 {
            kernel
                .advance(&[])
                .expect("a faulting handler must not stop the tick");
        }
        let subsystem = scripts(&kernel);
        assert!(subsystem.is_disabled(0), "the runaway is out of the run");
        assert!(!subsystem.is_disabled(1));
        assert_eq!(
            subsystem.faulted(),
            1,
            "it faults once and is not raised to again"
        );
        assert_eq!(
            subsystem.mission().counter("ticks"),
            4,
            "the sound script keeps running"
        );

        let fault = subsystem.faults()[0].clone();
        assert_eq!(fault.path, "scripts/runaway.cics");
        assert_eq!(fault.event, "tick");
        assert_eq!(fault.tick, 0);
        assert!(
            fault.detail.contains("instructions"),
            "the diagnostic names the fuel limit: {}",
            fault.detail
        );
    }

    #[test]
    fn a_counter_that_would_overflow_faults_rather_than_saturating() {
        let mut kernel = kernel(&[(
            "scripts/mission.cics",
            r#"
            on start() { sys.add_counter("n", 9223372036854775807); }
            on tick(e) { sys.add_counter("n", 1); }
            "#,
        )]);
        kernel.advance(&[]).expect("advances");
        let subsystem = scripts(&kernel);
        assert_eq!(subsystem.faulted(), 1);
        assert!(subsystem.is_disabled(0));
        assert_eq!(
            subsystem.mission().counter("n"),
            i64::MAX,
            "the counter keeps the last value it actually reached"
        );
    }

    #[test]
    fn logging_is_retained_up_to_a_bound_and_counted_without_one() {
        let mut kernel = kernel(&[("scripts/mission.cics", r#"on tick(e) { sys.log("beat"); }"#)]);
        let ticks = super::MESSAGES_RETAINED + 20;
        for _ in 0..ticks {
            kernel.advance(&[]).expect("advances");
        }
        let mission = scripts(&kernel).mission();
        assert_eq!(mission.messages_logged(), ticks as u64);
        assert_eq!(
            mission.messages().len(),
            super::MESSAGES_RETAINED,
            "the retained text is bounded so a long match does not grow it forever"
        );
    }

    #[test]
    fn the_dispatch_replays_identically() {
        let sources = [
            (
                "scripts/mission.cics",
                r#"
                on start() { sys.arm_timer("wave", 0.5); sys.set_flag("started", true); }
                on tick(elapsed) {
                    sys.add_counter("ticks", 1);
                    if sys.counter("ticks") == 20 { sys.arm_timer("second", 0.25); }
                    sys.log("beat");
                }
                on timer_elapsed(timer) {
                    sys.add_counter("timers", 1);
                    if timer == "wave" { sys.arm_timer("wave", 0.5); }
                }
                "#,
            ),
            (
                "scripts/observer.cics",
                r#"on tick(e) { if sys.flag("started") { sys.add_counter("seen", 1); } }"#,
            ),
        ];
        let run = || {
            let mut kernel = kernel(&sources);
            (0..90)
                .map(|_| kernel.advance(&[]).expect("advances"))
                .collect::<Vec<_>>()
        };
        let ours = run();
        let theirs = run();
        assert_eq!(first_divergence(&ours, &theirs), None);
        assert_eq!(ours, theirs);
    }

    #[test]
    fn mission_state_is_inside_the_hash() {
        // The point of ADR 7002 decision 7: what a script remembers is hashed state, so a machine
        // whose scripts did something different diverges on the tick it happened rather than drifting
        // silently.
        let plain = [(
            "scripts/mission.cics",
            r#"on tick(e) { sys.add_counter("n", 1); }"#,
        )];
        let extra = [(
            "scripts/mission.cics",
            r#"on tick(e) { sys.add_counter("n", 1); if sys.counter("n") == 3 { sys.set_flag("odd", true); } }"#,
        )];
        let run = |sources: &[(&str, &str)]| {
            let mut kernel = kernel(sources);
            (0..5)
                .map(|_| kernel.advance(&[]).expect("advances"))
                .collect::<Vec<_>>()
        };
        let divergence =
            first_divergence(&run(&plain), &run(&extra)).expect("the extra flag must be seen");
        assert_eq!(divergence.tick, 2, "the tick the flag was set");
        assert_eq!(divergence.entry, Some(SCRIPTS));
    }

    #[test]
    fn a_scenario_with_no_scripts_ticks_without_incident() {
        let mut kernel = kernel(&[]);
        kernel.advance(&[]).expect("advances");
        assert!(scripts(&kernel).paths().is_empty());
        assert_eq!(scripts(&kernel).faulted(), 0);
    }
}
