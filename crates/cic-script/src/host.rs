//! What the engine offers a script, and the reason a script cannot reach anything else.
//!
//! # The closed surface, which is the security model
//!
//! A script may call two things: functions it defines itself, and the host functions named in an
//! [`Interface`]. There is no reflection, no dynamic lookup, no module system and no way to name a
//! symbol that is not in that list. `sys.grant_resources(...)` in a downloaded mod does not fail at
//! run time when somebody triggers it — it fails to **compile**, naming the file and the line.
//!
//! This is [the interface layer's action-set rule](../../cic-ui/src/action.rs) applied one layer down,
//! and it is the same argument: a handler name looked up in a table at call time defers a typo to the
//! worst possible moment, and once mods can supply scripts, a string is an open channel into whatever
//! the table happens to contain.
//!
//! The same closure covers **events**. A script declares `on tick(elapsed)`, and if the engine defines
//! no `tick` event, or defines it with a different number of parameters, that is a compile error too.
//! Without it, a handler with a misspelled name is a handler that simply never runs, which is
//! indistinguishable from one whose body is wrong.
//!
//! # The standard functions are here rather than in the language
//!
//! `sqrt`, `sin` and the rest are host functions, not operators, so a host that has no business
//! offering trigonometry can decline to. They are implemented in [`cic_math`] rather than by calling
//! the platform's, which is the whole point: [ADR
//! 0007](../../../docs/adr/0007-simulation-arithmetic.md) forbids a platform transcendental anywhere
//! near simulation state, because the standard only *recommends* correct rounding for those and two
//! conforming libraries disagree in the last bits.
//!
//! **`sys.sin` and `sys.cos` take turns, not radians**, following ADR 0007 decision 5 — one full
//! revolution is `1.0`, so `sys.sin(0.25)` is one. The reduction of a large angle then becomes an exact
//! subtraction instead of the most delicate part of a libm.

use crate::value::{Value, type_name};
use cic_math::{ArithmeticError, cos_turns, sin_turns, sqrt};

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// A callable's name and how many arguments it takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// The name a script writes.
    pub name: String,
    /// How many arguments it takes.
    pub arity: u8,
}

/// A failure declaring something on an interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterfaceError {
    /// The name was already declared.
    Duplicate {
        /// The name.
        name: String,
    },
    /// More were declared than a handle can address.
    TooMany,
}

impl Display for InterfaceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duplicate { name } => write!(formatter, "`{name}` is already declared"),
            Self::TooMany => write!(formatter, "more entries than an index can address"),
        }
    }
}

impl Error for InterfaceError {}

/// Everything the engine exposes: the host functions a script may call and the events it may handle.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Interface {
    functions: Vec<Signature>,
    function_index: BTreeMap<String, u16>,
    events: Vec<Signature>,
    event_index: BTreeMap<String, u16>,
}

impl Interface {
    /// An interface offering nothing at all.
    ///
    /// The right starting point: a host adds what it means to expose, rather than removing what it
    /// forgot it was exposing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares a host function, returning the index a compiled call will carry.
    ///
    /// # Errors
    ///
    /// Returns [`InterfaceError`] for a repeated name or for more functions than an index can address.
    pub fn declare_function(
        &mut self,
        name: impl Into<String>,
        arity: u8,
    ) -> Result<u16, InterfaceError> {
        let name = name.into();
        if self.function_index.contains_key(&name) {
            return Err(InterfaceError::Duplicate { name });
        }
        let index = u16::try_from(self.functions.len()).map_err(|_| InterfaceError::TooMany)?;
        self.function_index.insert(name.clone(), index);
        self.functions.push(Signature { name, arity });
        Ok(index)
    }

    /// Declares an event a script may handle.
    ///
    /// # Errors
    ///
    /// Returns [`InterfaceError`] for a repeated name or for more events than an index can address.
    pub fn declare_event(
        &mut self,
        name: impl Into<String>,
        arity: u8,
    ) -> Result<u16, InterfaceError> {
        let name = name.into();
        if self.event_index.contains_key(&name) {
            return Err(InterfaceError::Duplicate { name });
        }
        let index = u16::try_from(self.events.len()).map_err(|_| InterfaceError::TooMany)?;
        self.event_index.insert(name.clone(), index);
        self.events.push(Signature { name, arity });
        Ok(index)
    }

    /// Looks a host function up by name.
    #[must_use]
    pub fn function(&self, name: &str) -> Option<(u16, u8)> {
        let index = *self.function_index.get(name)?;
        Some((index, self.functions[index as usize].arity))
    }

    /// Looks an event up by name.
    #[must_use]
    pub fn event(&self, name: &str) -> Option<(u16, u8)> {
        let index = *self.event_index.get(name)?;
        Some((index, self.events[index as usize].arity))
    }

    /// Every host function name, for a diagnostic that lists what *was* available.
    #[must_use]
    pub fn function_names(&self) -> Vec<&str> {
        self.functions
            .iter()
            .map(|signature| signature.name.as_str())
            .collect()
    }

    /// Every event name.
    #[must_use]
    pub fn event_names(&self) -> Vec<&str> {
        self.events
            .iter()
            .map(|signature| signature.name.as_str())
            .collect()
    }

    /// An interface offering the standard functions and nothing else.
    ///
    /// A host adds its own game verbs on top; this only provides the arithmetic a script cannot write
    /// for itself without reaching a platform routine that is not reproducible.
    ///
    /// # Panics
    ///
    /// Panics if [`STANDARD`] holds a repeated name, which is a fault in this file rather than in a
    /// caller and is caught by a test.
    #[must_use]
    pub fn standard() -> Self {
        let mut interface = Self::new();
        for (name, arity) in STANDARD {
            interface
                .declare_function(name, arity)
                .expect("the standard set has no repeated names");
        }
        interface
    }
}

/// 2^63, the first `f64` that is too large to be an `i64`.
///
/// Written out rather than computed with `powi`, which ADR 0007 forbids — not because it is inexact
/// but because its lowering is unspecified. 2^63 is exactly representable, so the literal is the same
/// number with no instruction to argue about. The textual guard in `tests/arithmetic_restriction.rs`
/// caught the first draft of this, which is the whole reason that test exists.
const LIMIT: f64 = 9_223_372_036_854_775_808.0;

/// The standard host functions, in the order their indices are assigned.
///
/// One table, read by both [`Interface::standard`] and [`StandardHost`], so a function cannot be
/// declared at one index and implemented at another. A test asserts the two agree.
pub const STANDARD: [(&str, u8); 9] = [
    ("abs", 1),
    ("min", 2),
    ("max", 2),
    ("clamp", 3),
    ("sqrt", 1),
    ("sin", 1),
    ("cos", 1),
    ("floor", 1),
    ("log", 1),
];

/// A call arriving at the host.
#[derive(Debug, Clone, Copy)]
pub struct HostCall<'a> {
    /// Which function, as an index into the interface it was compiled against.
    pub index: u16,
    /// The arguments, already checked to be the declared number of them.
    pub arguments: &'a [Value],
    /// The program's string table, for resolving a [`Value::Str`].
    pub strings: &'a [String],
}

impl HostCall<'_> {
    /// Reads an argument as a real, promoting an integer.
    ///
    /// # Errors
    ///
    /// Returns [`HostError::Type`] when the argument is not numeric.
    #[expect(
        clippy::cast_precision_loss,
        reason = "an exactly specified, correctly rounded conversion that ADR 0007 permits by name"
    )]
    pub fn number(&self, position: usize, function: &'static str) -> Result<f64, HostError> {
        match self.arguments.get(position) {
            Some(Value::Real(value)) => Ok(*value),
            Some(Value::Int(value)) => Ok(*value as f64),
            other => Err(HostError::Type {
                function,
                position,
                expected: "a number",
                found: other.map_or("nothing", |value| type_name(*value)),
            }),
        }
    }

    /// Reads an argument as text.
    ///
    /// # Errors
    ///
    /// Returns [`HostError::Type`] when the argument is not a string.
    pub fn text(&self, position: usize, function: &'static str) -> Result<&str, HostError> {
        match self.arguments.get(position) {
            Some(Value::Str(index)) => {
                self.strings
                    .get(*index as usize)
                    .map(String::as_str)
                    .ok_or(HostError::Message {
                        detail: "string index is outside the program's table".to_owned(),
                    })
            }
            other => Err(HostError::Type {
                function,
                position,
                expected: "a string",
                found: other.map_or("nothing", |value| type_name(*value)),
            }),
        }
    }
}

/// A failure inside a host function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    /// The arithmetic failed.
    Arithmetic(ArithmeticError),
    /// An argument was of the wrong type.
    Type {
        /// The function's name.
        function: &'static str,
        /// Which argument, counting from zero.
        position: usize,
        /// What it should have been.
        expected: &'static str,
        /// What it was.
        found: &'static str,
    },
    /// The host called an index it does not implement.
    ///
    /// Reachable only by compiling against one interface and running against a different host, which
    /// is a host bug — but it is reported rather than panicking, because a script must not be able to
    /// take the process down whatever anybody got wrong.
    Unimplemented {
        /// The index that arrived.
        index: u16,
    },
    /// Anything else the host wants to say.
    Message {
        /// What went wrong.
        detail: String,
    },
}

impl Display for HostError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Arithmetic(error) => write!(formatter, "{error}"),
            Self::Type {
                function,
                position,
                expected,
                found,
            } => write!(
                formatter,
                "`sys.{function}` argument {position} should be {expected}, found `{found}`"
            ),
            Self::Unimplemented { index } => {
                write!(
                    formatter,
                    "the host does not implement function index {index}"
                )
            }
            Self::Message { detail } => write!(formatter, "{detail}"),
        }
    }
}

impl Error for HostError {}

impl From<ArithmeticError> for HostError {
    fn from(error: ArithmeticError) -> Self {
        Self::Arithmetic(error)
    }
}

/// What the virtual machine calls out to.
pub trait Host {
    /// Runs one host function.
    ///
    /// # Errors
    ///
    /// Returns [`HostError`] for a bad argument or anything the host wants to refuse.
    fn call(&mut self, call: HostCall<'_>) -> Result<Value, HostError>;
}

/// An implementation of [`STANDARD`].
///
/// Collects what `sys.log` was given rather than printing it, because a test wanting to assert what a
/// script logged should not have to capture a stream, and a game wanting it on screen should not have
/// to intercept one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StandardHost {
    /// Everything `sys.log` has been given, in order.
    pub log: Vec<String>,
}

impl StandardHost {
    /// A host with an empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Host for StandardHost {
    fn call(&mut self, call: HostCall<'_>) -> Result<Value, HostError> {
        let name = STANDARD
            .get(call.index as usize)
            .map(|(name, _)| *name)
            .ok_or(HostError::Unimplemented { index: call.index })?;

        let value = match name {
            "abs" => Value::Real(call.number(0, "abs")?.abs()),
            "min" => Value::Real(call.number(0, "min")?.min(call.number(1, "min")?)),
            "max" => Value::Real(call.number(0, "max")?.max(call.number(1, "max")?)),
            "clamp" => {
                let low = call.number(1, "clamp")?;
                let high = call.number(2, "clamp")?;
                // A reversed range returns `low` rather than panicking, which `f64::clamp` does.
                let value = call.number(0, "clamp")?;
                Value::Real(if high < low {
                    low
                } else {
                    value.max(low).min(high)
                })
            }
            "sqrt" => Value::Real(sqrt(call.number(0, "sqrt")?)?),
            "sin" => Value::Real(sin_turns(call.number(0, "sin")?)),
            "cos" => Value::Real(cos_turns(call.number(0, "cos")?)),
            // `floor` returns an integer rather than a whole real, because what a caller wants it for
            // is an index, and handing back a real would mean every use site converts.
            "floor" => {
                let value = call.number(0, "floor")?.floor();
                // Outside `i64` there is no index to return, and saturating would hand back a
                // plausible number that is wrong.
                if !(-LIMIT..LIMIT).contains(&value) {
                    return Err(HostError::Arithmetic(ArithmeticError::IntegerOverflow));
                }
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "range checked immediately above"
                )]
                Value::Int(value as i64)
            }
            "log" => {
                self.log.push(call.text(0, "log")?.to_owned());
                Value::Nil
            }
            _ => return Err(HostError::Unimplemented { index: call.index }),
        };
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{Host, HostCall, HostError, Interface, InterfaceError, STANDARD, StandardHost};
    use crate::value::Value;

    #[test]
    fn the_standard_table_is_the_only_source_of_indices() {
        // Declared at one index and implemented at another is exactly the bug two tables would allow,
        // so this asserts the one table drives both.
        let interface = Interface::standard();
        for (position, (name, arity)) in STANDARD.iter().enumerate() {
            let (index, declared) = interface.function(name).expect("declared");
            assert_eq!(usize::from(index), position, "`{name}` moved");
            assert_eq!(declared, *arity);
        }
        assert_eq!(interface.function_names().len(), STANDARD.len());
    }

    #[test]
    fn an_empty_interface_offers_nothing() {
        // A host adds what it means to expose rather than removing what it forgot it was exposing.
        let interface = Interface::new();
        assert!(interface.function("sqrt").is_none());
        assert!(interface.event("tick").is_none());
    }

    #[test]
    fn a_repeated_declaration_is_refused() {
        let mut interface = Interface::new();
        assert_eq!(interface.declare_function("spawn", 2), Ok(0));
        assert_eq!(
            interface.declare_function("spawn", 3),
            Err(InterfaceError::Duplicate {
                name: "spawn".to_owned()
            })
        );
        // Functions and events are separate namespaces, so a `tick` event and a `tick` function do
        // not collide -- they are written differently and resolved differently.
        assert_eq!(interface.declare_event("spawn", 0), Ok(0));
    }

    fn call(host: &mut StandardHost, name: &str, arguments: &[Value]) -> Result<Value, HostError> {
        let index = Interface::standard().function(name).expect("declared").0;
        host.call(HostCall {
            index,
            arguments,
            strings: &["hello".to_owned()],
        })
    }

    #[test]
    fn the_standard_functions_compute_what_they_claim() {
        let mut host = StandardHost::new();
        assert_eq!(
            call(&mut host, "sqrt", &[Value::Int(4)]),
            Ok(Value::Real(2.0))
        );
        assert_eq!(
            call(&mut host, "abs", &[Value::Int(-3)]),
            Ok(Value::Real(3.0))
        );
        assert_eq!(
            call(&mut host, "min", &[Value::Int(2), Value::Int(7)]),
            Ok(Value::Real(2.0))
        );
        assert_eq!(
            call(
                &mut host,
                "clamp",
                &[Value::Int(9), Value::Int(0), Value::Int(5)]
            ),
            Ok(Value::Real(5.0))
        );
    }

    #[test]
    fn the_angle_functions_take_turns_rather_than_radians() {
        // ADR 0007 decision 5. A quarter turn is one, and a script never has to write pi.
        let mut host = StandardHost::new();
        assert_eq!(
            call(&mut host, "sin", &[Value::Int(0)]),
            Ok(Value::Real(0.0))
        );

        let Ok(Value::Real(quarter)) = call(&mut host, "sin", &[Value::Real(0.25)]) else {
            panic!("expected a real");
        };
        assert!(
            (quarter - 1.0).abs() < 1e-15,
            "sine of a quarter turn was {quarter}"
        );

        let Ok(Value::Real(full)) = call(&mut host, "cos", &[Value::Real(1.0)]) else {
            panic!("expected a real");
        };
        assert!(
            (full - 1.0).abs() < 1e-15,
            "cosine of a full turn was {full}"
        );
    }

    #[test]
    fn a_reversed_clamp_range_returns_a_bound_rather_than_panicking() {
        // `f64::clamp` panics when its bounds arrive the wrong way round, and these come from a script.
        let mut host = StandardHost::new();
        assert_eq!(
            call(
                &mut host,
                "clamp",
                &[Value::Int(5), Value::Int(10), Value::Int(2)]
            ),
            Ok(Value::Real(10.0))
        );
    }

    #[test]
    fn floor_returns_an_integer_because_what_it_is_for_is_an_index() {
        let mut host = StandardHost::new();
        assert_eq!(
            call(&mut host, "floor", &[Value::Real(-0.5)]),
            Ok(Value::Int(-1))
        );
        assert_eq!(
            call(&mut host, "floor", &[Value::Real(2.75)]),
            Ok(Value::Int(2))
        );
    }

    #[test]
    fn flooring_something_outside_the_integer_range_is_a_fault() {
        // Saturating would hand back a plausible index that is wrong.
        let mut host = StandardHost::new();
        assert!(call(&mut host, "floor", &[Value::Real(1e30)]).is_err());
    }

    #[test]
    fn log_collects_rather_than_printing() {
        let mut host = StandardHost::new();
        assert_eq!(call(&mut host, "log", &[Value::Str(0)]), Ok(Value::Nil));
        assert_eq!(host.log, vec!["hello".to_owned()]);
    }

    #[test]
    fn an_argument_of_the_wrong_type_names_the_function_and_the_position() {
        let mut host = StandardHost::new();
        let error = call(&mut host, "sqrt", &[Value::Bool(true)]).expect_err("refuse");
        assert_eq!(
            error.to_string(),
            "`sys.sqrt` argument 0 should be a number, found `bool`"
        );
    }

    #[test]
    fn a_negative_square_root_is_an_error_rather_than_a_panic() {
        // A script running inside a simulation tick must not be able to take the process down.
        let mut host = StandardHost::new();
        assert!(call(&mut host, "sqrt", &[Value::Int(-1)]).is_err());
    }

    #[test]
    fn an_index_the_host_does_not_implement_is_reported_rather_than_panicking() {
        // Only reachable by compiling against one interface and running against another, which is a
        // host bug -- but a script must not be able to abort the process whoever got it wrong.
        let mut host = StandardHost::new();
        let error = host
            .call(HostCall {
                index: 9_000,
                arguments: &[],
                strings: &[],
            })
            .expect_err("refuse");
        assert_eq!(error, HostError::Unimplemented { index: 9_000 });
    }

    #[test]
    fn a_string_index_outside_the_table_is_an_error() {
        let mut host = StandardHost::new();
        let error = host
            .call(HostCall {
                index: 8,
                arguments: &[Value::Str(99)],
                strings: &[],
            })
            .expect_err("refuse");
        assert!(matches!(error, HostError::Message { .. }));
    }
}
