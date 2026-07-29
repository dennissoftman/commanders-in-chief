//! What a script can hold, and what arithmetic on it does.
//!
//! # There is no heap, and that is the design rather than a limitation
//!
//! Every value is a fixed-size scalar. A string is an *index* into the program's constant table, so a
//! script can pass text around and compare it but cannot build any, and the set of strings a program
//! can ever hold is fixed when it compiles.
//!
//! Three things follow, and all three are requirements rather than conveniences:
//!
//! - **No allocator in the interpreter**, so a script cannot exhaust memory however hostile it is. The
//!   fuel limit bounds time and the absence of a heap bounds space, which between them make "run
//!   untrusted content inside a simulation tick" a defensible claim.
//! - **No garbage collector**, so there is no collection pause and no allocation order to be
//!   non-deterministic about.
//! - **The whole value type is `Copy`**, so the stack is a `Vec` of scalars and nothing in the
//!   interpreter has a lifetime.
//!
//! The cost is real: no lists, no maps, no string building. That is recorded as a deliberate omission
//! in [the milestone](../../../docs/milestones/m10-scripting.md), along with what would have to change
//! to lift it.
//!
//! # Numbers follow ADR 0007
//!
//! [`Value::Real`] is an `f64` and every operation on it is in the permitted set — see [`cic_math`].
//! Mixed arithmetic promotes toward it: `2 * 1.5` is `3.0`, because truncating the other way would
//! discard a fraction an author wrote deliberately.
//!
//! **Two integers stay an integer**, and that is not merely an optimisation. `f64` represents every
//! integer exactly only up to 2^53, so a script counting in whole numbers is better served by `i64`
//! arithmetic that reports its own overflow than by one that silently loses the low bit at nine
//! quadrillion.

use cic_math::{ArithmeticError, finite};

use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// A value a script can hold.
///
/// Deliberately **not** `Eq`. `f64` is not, and pretending otherwise would hide the one place it
/// matters — constant interning, which compares bit patterns rather than values so that two spellings
/// of the same literal share an entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    /// The absence of a value, which is what a function without a `return` produces.
    Nil,
    /// A truth value.
    Bool(bool),
    /// A whole number.
    Int(i64),
    /// A real number, in the arithmetic ADR 0007 pins.
    Real(f64),
    /// Text, as an index into the program's string table.
    Str(u16),
}

/// The name of a value's type, for an error message.
#[must_use]
pub const fn type_name(value: Value) -> &'static str {
    match value {
        Value::Nil => "nil",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Real(_) => "real",
        Value::Str(_) => "str",
    }
}

/// Whether two values are the same constant, comparing reals by bit pattern.
///
/// Used for interning. Bit comparison rather than `==`, so that a literal `0.0` and a literal `-0.0`
/// stay distinct — they divide differently, so folding them together would change what a program means.
#[must_use]
pub fn same_constant(left: Value, right: Value) -> bool {
    match (left, right) {
        (Value::Real(left), Value::Real(right)) => left.to_bits() == right.to_bits(),
        _ => left == right,
    }
}

/// Why an operation on a value failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueError {
    /// An operator was applied to a type it is not defined for.
    Type {
        /// The operator, as it is written.
        operator: &'static str,
        /// Type of the left operand, or of the only one.
        left: &'static str,
        /// Type of the right operand, or `None` for a unary operator.
        right: Option<&'static str>,
    },
    /// The arithmetic itself failed.
    Arithmetic(ArithmeticError),
}

impl Display for ValueError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Type {
                operator,
                left,
                right,
            } => match right {
                Some(right) => write!(
                    formatter,
                    "`{operator}` is not defined for `{left}` and `{right}`"
                ),
                None => write!(formatter, "`{operator}` is not defined for `{left}`"),
            },
            Self::Arithmetic(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for ValueError {}

impl From<ArithmeticError> for ValueError {
    fn from(error: ArithmeticError) -> Self {
        Self::Arithmetic(error)
    }
}

impl Value {
    /// Whether the value counts as true in a condition.
    ///
    /// Only `Bool` does. Not zero, not nil, not the empty string — because "truthiness" is the source
    /// of a class of bug where a value of the wrong type takes a branch instead of being reported, and
    /// this language is read by people writing game logic rather than by people who enjoy coercion
    /// tables.
    ///
    /// # Errors
    ///
    /// Returns [`ValueError::Type`] for anything that is not a `Bool`.
    pub const fn as_bool(self, operator: &'static str) -> Result<bool, ValueError> {
        match self {
            Self::Bool(value) => Ok(value),
            other => Err(ValueError::Type {
                operator,
                left: type_name(other),
                right: None,
            }),
        }
    }

    /// The value as a real, promoting an integer.
    ///
    /// The `i64` to `f64` conversion is on ADR 0007's permitted list and is exactly specified — it
    /// rounds to nearest, identically everywhere — so it stays deterministic where it is lossy above
    /// 2^53.
    #[expect(
        clippy::cast_precision_loss,
        reason = "an exactly specified, correctly rounded conversion that ADR 0007 permits by name; \
                  determinism does not require it to be lossless"
    )]
    fn as_real(self) -> Option<f64> {
        match self {
            Self::Real(value) => Some(value),
            Self::Int(value) => Some(value as f64),
            _ => None,
        }
    }
}

/// Applies a binary arithmetic operator.
///
/// # Errors
///
/// Returns [`ValueError`] for operands of the wrong type, for a non-finite result, for integer
/// overflow, or for division by zero.
pub fn arithmetic(operator: &'static str, left: Value, right: Value) -> Result<Value, ValueError> {
    let mismatch = || ValueError::Type {
        operator,
        left: type_name(left),
        right: Some(type_name(right)),
    };

    if let (Value::Int(left), Value::Int(right)) = (left, right) {
        let result = match operator {
            "+" => left.checked_add(right),
            "-" => left.checked_sub(right),
            "*" => left.checked_mul(right),
            "/" => {
                if right == 0 {
                    return Err(ArithmeticError::DivideByZero.into());
                }
                left.checked_div(right)
            }
            "%" => {
                if right == 0 {
                    return Err(ArithmeticError::DivideByZero.into());
                }
                left.checked_rem(right)
            }
            _ => return Err(mismatch()),
        };
        return result
            .map(Value::Int)
            .ok_or_else(|| ArithmeticError::IntegerOverflow.into());
    }

    let (Some(left), Some(right)) = (left.as_real(), right.as_real()) else {
        return Err(mismatch());
    };
    // Checked before the operation rather than inferred from an infinity afterwards, so the diagnostic
    // names what happened. `1.0 / 0.0` is a well-defined infinity in IEEE-754 and a mistake in a script.
    if matches!(operator, "/" | "%") && right == 0.0 {
        return Err(ArithmeticError::DivideByZero.into());
    }
    let result = match operator {
        "+" => left + right,
        "-" => left - right,
        "*" => left * right,
        "/" => left / right,
        "%" => left % right,
        _ => return Err(mismatch()),
    };
    finite(result).map(Value::Real).map_err(Into::into)
}

/// Applies unary negation.
///
/// # Errors
///
/// Returns [`ValueError`] for a non-numeric operand or for integer overflow.
pub fn negate(value: Value) -> Result<Value, ValueError> {
    match value {
        Value::Int(inner) => inner
            .checked_neg()
            .map(Value::Int)
            .ok_or_else(|| ArithmeticError::IntegerOverflow.into()),
        Value::Real(inner) => Ok(Value::Real(-inner)),
        other => Err(ValueError::Type {
            operator: "-",
            left: type_name(other),
            right: None,
        }),
    }
}

/// Compares two values with an ordering operator.
///
/// # Errors
///
/// Returns [`ValueError::Type`] for operands that cannot be ordered.
pub fn compare(operator: &'static str, left: Value, right: Value) -> Result<Value, ValueError> {
    let (Some(left), Some(right)) = (left.as_real(), right.as_real()) else {
        return Err(ValueError::Type {
            operator,
            left: type_name(left),
            right: Some(type_name(right)),
        });
    };
    let outcome = match operator {
        "<" => left < right,
        "<=" => left <= right,
        ">" => left > right,
        ">=" => left >= right,
        _ => {
            return Err(ValueError::Type {
                operator,
                left: "real",
                right: Some("real"),
            });
        }
    };
    Ok(Value::Bool(outcome))
}

/// Whether two values are equal.
///
/// Cross-type numeric equality holds — `1 == 1.0` is true — because a script author comparing a count
/// against a threshold should not have to know which of the two the engine handed them. Values of
/// unrelated types are simply unequal rather than an error, so a `nil` check is expressible.
#[must_use]
pub fn equals(left: Value, right: Value) -> bool {
    match (left, right) {
        (Value::Nil, Value::Nil) => true,
        (Value::Bool(left), Value::Bool(right)) => left == right,
        // Two strings are equal when their indices are, which works because the compiler interns the
        // constant table -- two identical literals become one entry. Without that interning this would
        // silently report equal strings as different.
        (Value::Str(left), Value::Str(right)) => left == right,
        _ => match (left.as_real(), right.as_real()) {
            #[expect(
                clippy::float_cmp,
                reason = "exact equality is the semantics `==` has in this language, and it is the                           only one that can be: a tolerance would make equality non-transitive, so                           `a == b` and `b == c` would no longer imply `a == c` and a script's                           conditions would stop composing"
            )]
            (Some(left), Some(right)) => left == right,
            _ => false,
        },
    }
}

/// Renders a value for a diagnostic, resolving a string index against `strings`.
#[must_use]
pub fn display(value: Value, strings: &[String]) -> String {
    match value {
        Value::Nil => "nil".to_owned(),
        Value::Bool(inner) => inner.to_string(),
        Value::Int(inner) => inner.to_string(),
        // The `Debug` form of an `f64` round-trips exactly, which is what a diagnostic about a
        // determinism-sensitive value has to do -- `Display` would print `0.1` for several distinct
        // numbers.
        Value::Real(inner) => format!("{inner:?}"),
        Value::Str(index) => strings
            .get(index as usize)
            .cloned()
            .unwrap_or_else(|| "<unknown string>".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::{Value, ValueError, arithmetic, compare, display, equals, negate, same_constant};
    use cic_math::ArithmeticError;

    #[test]
    fn two_integers_stay_an_integer() {
        // Not an optimisation: `f64` represents integers exactly only to 2^53, so whole-number counting
        // is better served by `i64` arithmetic that reports its own overflow.
        assert_eq!(
            arithmetic("+", Value::Int(2), Value::Int(3)),
            Ok(Value::Int(5))
        );
        assert_eq!(
            arithmetic("/", Value::Int(7), Value::Int(2)),
            Ok(Value::Int(3)),
            "integer division truncates rather than promoting"
        );
    }

    #[test]
    fn mixing_an_integer_and_a_real_promotes_toward_the_real() {
        assert_eq!(
            arithmetic("*", Value::Int(2), Value::Real(1.5)),
            Ok(Value::Real(3.0))
        );
        assert_eq!(
            arithmetic("+", Value::Real(0.5), Value::Int(1)),
            Ok(Value::Real(1.5))
        );
    }

    #[test]
    fn integer_overflow_and_division_by_zero_are_errors_rather_than_panics() {
        assert_eq!(
            arithmetic("+", Value::Int(i64::MAX), Value::Int(1)),
            Err(ValueError::Arithmetic(ArithmeticError::IntegerOverflow))
        );
        assert_eq!(
            arithmetic("/", Value::Int(1), Value::Int(0)),
            Err(ValueError::Arithmetic(ArithmeticError::DivideByZero))
        );
        assert_eq!(
            negate(Value::Int(i64::MIN)),
            Err(ArithmeticError::IntegerOverflow.into())
        );
    }

    #[test]
    fn dividing_a_real_by_zero_is_a_fault_rather_than_an_infinity() {
        // IEEE-754 defines it, and it is still a mistake in a script. Reported by name rather than as
        // "not finite", because a diagnostic should say what happened.
        assert_eq!(
            arithmetic("/", Value::Real(1.0), Value::Real(0.0)),
            Err(ValueError::Arithmetic(ArithmeticError::DivideByZero))
        );
        assert_eq!(
            arithmetic("%", Value::Real(1.0), Value::Int(0)),
            Err(ValueError::Arithmetic(ArithmeticError::DivideByZero))
        );
    }

    #[test]
    fn an_overflowing_real_is_a_fault_rather_than_an_infinity() {
        // The reason: every comparison against a NaN is false, so a rule involving one silently does
        // not fire. An infinity becomes a NaN one subtraction later.
        assert_eq!(
            arithmetic("*", Value::Real(f64::MAX), Value::Real(2.0)),
            Err(ValueError::Arithmetic(ArithmeticError::NonFinite))
        );
    }

    #[test]
    fn arithmetic_on_a_non_number_names_both_types() {
        let error = arithmetic("+", Value::Bool(true), Value::Int(1)).expect_err("refuse");
        assert_eq!(
            error,
            ValueError::Type {
                operator: "+",
                left: "bool",
                right: Some("int")
            }
        );
        assert_eq!(error.to_string(), "`+` is not defined for `bool` and `int`");
    }

    #[test]
    fn only_a_bool_is_a_condition() {
        assert_eq!(Value::Bool(true).as_bool("if"), Ok(true));
        assert!(Value::Int(1).as_bool("if").is_err());
        assert!(Value::Int(0).as_bool("if").is_err());
        assert!(Value::Nil.as_bool("if").is_err());
        assert!(Value::Real(1.0).as_bool("if").is_err());
    }

    #[test]
    fn numeric_equality_crosses_the_two_numeric_types() {
        assert!(equals(Value::Int(1), Value::Real(1.0)));
        assert!(!equals(Value::Int(1), Value::Real(1.5)));
        assert!(equals(Value::Nil, Value::Nil));
        assert!(equals(Value::Bool(false), Value::Bool(false)));
    }

    #[test]
    fn values_of_unrelated_types_are_unequal_rather_than_an_error() {
        assert!(!equals(Value::Nil, Value::Int(0)));
        assert!(!equals(Value::Bool(true), Value::Int(1)));
        assert!(!equals(Value::Str(0), Value::Int(0)));
    }

    #[test]
    fn constant_interning_compares_bits_so_signed_zeroes_stay_distinct() {
        // They divide differently, so folding them together would change what a program means.
        assert!(same_constant(Value::Real(1.5), Value::Real(1.5)));
        assert!(!same_constant(Value::Real(0.0), Value::Real(-0.0)));
        assert!(
            equals(Value::Real(0.0), Value::Real(-0.0)),
            "but they still compare equal, which is what IEEE-754 says"
        );
        assert!(same_constant(Value::Int(3), Value::Int(3)));
        assert!(!same_constant(Value::Int(3), Value::Real(3.0)));
    }

    #[test]
    fn ordering_works_across_numeric_types_and_refuses_others() {
        assert_eq!(
            compare("<", Value::Int(1), Value::Real(1.5)),
            Ok(Value::Bool(true))
        );
        assert_eq!(
            compare(">=", Value::Real(1.5), Value::Int(1)),
            Ok(Value::Bool(true))
        );
        assert!(compare("<", Value::Str(0), Value::Str(1)).is_err());
        assert!(compare("<", Value::Bool(true), Value::Bool(false)).is_err());
    }

    #[test]
    fn display_resolves_a_string_index_and_survives_a_bad_one() {
        let strings = vec!["hello".to_owned()];
        assert_eq!(display(Value::Str(0), &strings), "hello");
        assert_eq!(display(Value::Str(9), &strings), "<unknown string>");
        assert_eq!(display(Value::Int(-3), &strings), "-3");
        assert_eq!(display(Value::Real(0.5), &strings), "0.5");
        assert_eq!(display(Value::Nil, &strings), "nil");
    }
}
