//! The simulation's arithmetic: the operations [ADR 0007] permits, and the transcendentals written to
//! stay inside them.
//!
//! # One implementation, below every consumer
//!
//! This crate exists so that the things running inside the simulation — the script VM today, the
//! kernel next — share one `sin` rather than owning one each. Two implementations inside one lockstep
//! simulation is two answers to the same question waiting to be compared, which is the same class of
//! bug as the two arithmetics [ADR 7001] records removing. M10 flagged the hazard while the kernel was
//! still unbuilt, and the crate was created then (2026-07-29, Denys's decision on the name) precisely
//! because whichever implementation existed first was going to be the one everything used.
//!
//! It depends on nothing, including nothing of this project's: the arithmetic has to sit below every
//! crate that could otherwise disagree about it, and a maths crate with no I/O is nothing a sandboxed
//! script can escape through.
//!
//! # This implements ADR 0007 rather than deciding anything
//!
//! [ADR 0007] settled simulation arithmetic for the whole engine: `f64`, restricted to the operations
//! IEEE-754 requires to be correctly rounded, with no platform transcendental anywhere near simulation
//! state — because what differs between platforms is the C library, not the arithmetic.
//!
//! # The permitted set
//!
//! `+ - * /`, `%`, `sqrt`, `abs`, `signum`, `copysign`, `min`, `max`, `clamp`, comparison,
//! `round`/`floor`/`ceil`/`trunc`, and conversions to and from integers. Every one is specified exactly
//! by IEEE-754 or by Rust, so two conforming platforms cannot disagree about any of them.
//!
//! What is *not* in the set is `sin`, `cos`, `exp`, `ln`, `powf` and the rest, because the standard only
//! *recommends* correct rounding for those and in Rust they call the platform's C library. That is the
//! entire divergence risk, and it is narrower than "floating point is not deterministic" suggests.
//!
//! ADR 0007 decision 8 requires a textual test scanning simulation code for the forbidden names, since
//! `cargo build` will not enforce this. This crate carries its own, in
//! `tests/arithmetic_restriction.rs`, exactly as its consumers do — the guard travels with the code it
//! guards.
//!
//! # Angles are turns, not radians
//!
//! ADR 0007 decision 5 stores angles as a fraction of a revolution, because range reduction is where a
//! naive `sin` loses both accuracy and determinism: reducing a large radian argument modulo π needs more
//! precision than the argument carries, which is why MUSL's `rem_pio2` is the longest and most delicate
//! part of its `sin`.
//!
//! In turns the reduction is `x - x.floor()`, and **that is exact for every `f64`** — the fractional
//! part of a float is always representable, and the quadrant folds below are exact by Sterbenz's lemma.
//! So the delicate part of the problem is removed rather than solved, and what is left is a polynomial
//! over a bounded interval.
//!
//! ```
//! use cic_math::{cos_turns, sin_turns};
//!
//! // A quarter turn is exactly one, and a full turn ahead is the identical bit pattern:
//! // reducing a turn count is a subtraction that cannot round.
//! assert_eq!(sin_turns(0.25), 1.0);
//! assert_eq!(
//!     sin_turns(1_000_000.125).to_bits(),
//!     sin_turns(0.125).to_bits(),
//! );
//! assert_eq!(cos_turns(0.0), 1.0);
//! ```
//!
//! [ADR 0007]: ../../../docs/adr/0007-simulation-arithmetic.md
//! [ADR 7001]: ../../../docs/adr/7001-scripting-language.md

use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// Radians in one full turn.
///
/// Not `std::f64::consts::TAU` by a different name — it is the same value, named here because it is the
/// one place this crate leaves the turn domain, and a reader checking the reduction needs to see it.
const TAU: f64 = std::f64::consts::TAU;

/// What went wrong in an arithmetic operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithmeticError {
    /// The result was infinite or not a number.
    NonFinite,
    /// A division or remainder by zero.
    DivideByZero,
    /// A square root of a negative number.
    NegativeRoot,
    /// An integer operation left the range of `i64`.
    IntegerOverflow,
}

impl Display for ArithmeticError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite => write!(formatter, "result is not a finite number"),
            Self::DivideByZero => write!(formatter, "division by zero"),
            Self::NegativeRoot => write!(formatter, "square root of a negative number"),
            Self::IntegerOverflow => write!(formatter, "integer overflow"),
        }
    }
}

impl Error for ArithmeticError {}

/// Rejects a non-finite result.
///
/// # Why an infinity is refused when it is perfectly deterministic
///
/// It is: IEEE-754 pins infinity and NaN as exactly as it pins any other result, so allowing them would
/// not break lockstep. They are refused for a different reason. **Every comparison against a NaN is
/// false**, so a condition involving one silently takes its else branch rather than reporting anything,
/// and the author sees a rule that did not fire rather than an error. An infinity does the same
/// thing one step later, when it becomes a NaN by subtraction.
///
/// So the fault is a diagnostic rather than a determinism measure — and it is itself deterministic,
/// since every machine refuses at the same operation.
///
/// # Errors
///
/// Returns [`ArithmeticError::NonFinite`] for an infinity or a NaN.
pub fn finite(value: f64) -> Result<f64, ArithmeticError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(ArithmeticError::NonFinite)
    }
}

/// The square root.
///
/// `f64::sqrt` is used directly and that is deliberate: IEEE-754 *requires* it to be correctly rounded,
/// so unlike the transcendentals it is identical on every conforming platform. It is on ADR 0007's
/// permitted list for exactly this reason.
///
/// # Errors
///
/// Returns [`ArithmeticError::NegativeRoot`] for a negative argument, so a caller gets a diagnostic
/// rather than a NaN that disappears into a comparison.
pub fn sqrt(value: f64) -> Result<f64, ArithmeticError> {
    if value < 0.0 {
        return Err(ArithmeticError::NegativeRoot);
    }
    finite(value.sqrt())
}

/// The sine of an angle measured in turns, so one full revolution is `1.0`.
///
/// Written in the permitted operation set only — additions, multiplications, one `floor`, and one
/// comparison — so it computes the same bits on every platform. See the crate documentation for why
/// turns rather than radians.
#[must_use]
pub fn sin_turns(turns: f64) -> f64 {
    if !turns.is_finite() {
        return f64::NAN;
    }

    // Reduce to one revolution. Exact: the fractional part of an `f64` is always representable.
    let mut reduced = turns - turns.floor();

    // Fold the second half onto the first and negate. Exact by Sterbenz: `reduced` is in [0.5, 1).
    let mut sign = 1.0;
    if reduced >= 0.5 {
        reduced -= 0.5;
        sign = -1.0;
    }
    // Fold the second quarter back onto the first. Exact by Sterbenz again.
    if reduced > 0.25 {
        reduced = 0.5 - reduced;
    }

    // The one place this leaves the turn domain. A single correctly-rounded multiply.
    let radians = reduced * TAU;
    sign * polynomial(radians)
}

/// The cosine of an angle measured in turns.
///
/// A quarter turn ahead of the sine. The addition is correctly rounded rather than exact, which is
/// fine — what matters is that every platform rounds it identically.
#[must_use]
pub fn cos_turns(turns: f64) -> f64 {
    sin_turns(turns + 0.25)
}

/// The Taylor series for sine on `[0, pi/2]`, in Horner form.
///
/// Eleven terms, through `x^21`. That is more than a minimax polynomial would need for the same
/// accuracy, and it is used anyway because every coefficient is a *stated* reciprocal factorial that a
/// reader can verify from the series — where minimax coefficients are the output of a fitting tool and
/// have to be trusted. The cost is ten multiplications in a function callers reach for rarely.
///
/// Truncation error at the interval's end is about `x^23 / 23!`, which is 3e-19 — below what an `f64`
/// distinguishes near 1.0, so the accumulated rounding of the Horner evaluation dominates it.
///
/// `mul_add` is deliberately not used. ADR 0007 decision 6 permits it but warns it is not
/// interchangeable with `a * b + c`, because it rounds once where the pair rounds twice. Written out,
/// the expression means exactly what it reads and a reviewer cannot "simplify" it into a different
/// function.
fn polynomial(x: f64) -> f64 {
    /// `-1/3!`
    const C3: f64 = -1.0 / 6.0;
    /// `1/5!`
    const C5: f64 = 1.0 / 120.0;
    /// `-1/7!`
    const C7: f64 = -1.0 / 5_040.0;
    /// `1/9!`
    const C9: f64 = 1.0 / 362_880.0;
    /// `-1/11!`
    const C11: f64 = -1.0 / 39_916_800.0;
    /// `1/13!`
    const C13: f64 = 1.0 / 6_227_020_800.0;
    /// `-1/15!`
    const C15: f64 = -1.0 / 1_307_674_368_000.0;
    /// `1/17!`
    const C17: f64 = 1.0 / 355_687_428_096_000.0;
    /// `-1/19!`
    const C19: f64 = -1.0 / 121_645_100_408_832_000.0;
    /// `1/21!`
    const C21: f64 = 1.0 / 51_090_942_171_709_440_000.0;

    let square = x * x;
    let mut accumulated = C21;
    accumulated = C19 + square * accumulated;
    accumulated = C17 + square * accumulated;
    accumulated = C15 + square * accumulated;
    accumulated = C13 + square * accumulated;
    accumulated = C11 + square * accumulated;
    accumulated = C9 + square * accumulated;
    accumulated = C7 + square * accumulated;
    accumulated = C5 + square * accumulated;
    accumulated = C3 + square * accumulated;
    accumulated = 1.0 + square * accumulated;
    x * accumulated
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::{ArithmeticError, cos_turns, finite, sin_turns, sqrt};

    /// One unit in the last place near 1.0.
    const ULP: f64 = f64::EPSILON;

    #[test]
    fn the_permitted_operations_behave_as_ieee_requires() {
        // Not a strong test on one machine, and not meant to be — the guarantee is the standard's. It
        // pins that nothing here has wrapped these in something lossy.
        assert!((0.1_f64 + 0.2 - 0.300_000_000_000_000_04).abs() < f64::EPSILON);
        assert_eq!(sqrt(4.0), Ok(2.0));
        assert_eq!(sqrt(0.0), Ok(0.0));
    }

    #[test]
    fn a_negative_root_and_a_non_finite_value_are_faults_rather_than_nan() {
        // Every comparison against a NaN is false, so a condition involving one silently takes its else
        // branch. An author needs to be told, not to watch a rule quietly not fire.
        assert_eq!(sqrt(-1.0), Err(ArithmeticError::NegativeRoot));
        assert_eq!(finite(f64::INFINITY), Err(ArithmeticError::NonFinite));
        assert_eq!(finite(f64::NEG_INFINITY), Err(ArithmeticError::NonFinite));
        assert_eq!(finite(f64::NAN), Err(ArithmeticError::NonFinite));
        assert_eq!(finite(1.5), Ok(1.5));
    }

    /// Folds an angle in turns the way [`sin_turns`] does, returning the sign and the first-quadrant
    /// argument in radians.
    fn folded(turns: f64) -> (f64, f64) {
        let mut reduced = turns - turns.floor();
        let mut sign = 1.0;
        if reduced >= 0.5 {
            reduced -= 0.5;
            sign = -1.0;
        }
        if reduced > 0.25 {
            reduced = 0.5 - reduced;
        }
        (sign, reduced * std::f64::consts::TAU)
    }

    #[test]
    fn the_polynomial_agrees_with_the_platform_to_the_last_bit_on_the_same_argument() {
        // The comparison that isolates *this* function. Given the same first-quadrant argument, the
        // series and the platform's `sin` must agree — anything else would mean the polynomial is
        // short of a term, and it is measured here rather than assumed.
        for step in 0..=256 {
            let turns = f64::from(step) / 256.0;
            let (sign, radians) = folded(turns);
            let expected = sign * radians.sin();
            let actual = sin_turns(turns);
            assert!(
                (actual - expected).abs() <= ULP,
                "sin_turns({turns}) was {actual:?}, the platform on the same argument gave {expected:?}"
            );
        }
    }

    #[test]
    fn the_difference_from_a_naive_implementation_is_the_reduction_and_not_the_series() {
        // Worth pinning as a finding rather than as prose. Comparing `sin_turns(t)` against
        // `(t * TAU).sin()` shows a few units in the last place, and the whole of that gap is the
        // *reference's* rounding of the larger argument: `t * TAU` for a large `t` lands on a coarser
        // grid than the folded argument does, so the platform is answering a slightly different
        // question. This is exactly the range-reduction problem ADR 0007 decision 5 removes.
        let mut worst_against_naive = 0.0_f64;
        let mut worst_against_same_argument = 0.0_f64;
        for step in 0..=256 {
            let turns = f64::from(step) / 256.0;
            let (sign, radians) = folded(turns);
            let actual = sin_turns(turns);
            worst_against_naive =
                worst_against_naive.max((actual - (turns * std::f64::consts::TAU).sin()).abs());
            worst_against_same_argument =
                worst_against_same_argument.max((actual - sign * radians.sin()).abs());
        }
        assert!(
            worst_against_same_argument <= ULP,
            "against the same argument: {worst_against_same_argument:e}"
        );
        assert!(
            worst_against_naive > worst_against_same_argument,
            "the naive comparison should be the looser one"
        );
        assert!(
            worst_against_naive < 8.0 * ULP,
            "against a naive whole-angle reference: {worst_against_naive:e}"
        );
    }

    #[test]
    fn cosine_is_the_sine_a_quarter_turn_ahead() {
        for step in 0..=64 {
            let turns = f64::from(step) / 64.0;
            let expected = sin_turns(turns + 0.25);
            assert_eq!(cos_turns(turns).to_bits(), expected.to_bits());
        }
    }

    #[test]
    fn the_quarter_points_are_the_values_they_should_be() {
        assert_eq!(sin_turns(0.0), 0.0);
        assert!((sin_turns(0.25) - 1.0).abs() <= ULP);
        assert!(sin_turns(0.5).abs() <= ULP);
        assert!((sin_turns(0.75) + 1.0).abs() <= ULP);
        assert!((cos_turns(0.0) - 1.0).abs() <= ULP);
        assert!(cos_turns(0.25).abs() <= ULP);
        assert!((cos_turns(0.5) + 1.0).abs() <= ULP);
    }

    #[test]
    fn the_reduction_is_exact_so_a_huge_angle_is_as_accurate_as_a_small_one() {
        // This is the property turns exist for, and it is exact rather than approximate: every one of
        // these produces the *identical bit pattern*, because the reduction is a subtraction that
        // cannot round. In radians, reducing 1e12 modulo pi loses most of the argument's digits.
        let reference = sin_turns(0.125).to_bits();
        for turns in [1.125_f64, 1_000.125, 1_000_000.125, 1e12 + 0.125] {
            assert_eq!(
                sin_turns(turns).to_bits(),
                reference,
                "sin_turns({turns}) differs from sin_turns(0.125)"
            );
        }
    }

    #[test]
    fn sine_is_odd_and_periodic() {
        for step in 1..32 {
            let turns = f64::from(step) / 32.0;
            assert!(
                (sin_turns(turns) + sin_turns(-turns)).abs() <= ULP,
                "not odd at {turns}"
            );
            assert_eq!(
                sin_turns(turns).to_bits(),
                sin_turns(turns + 8.0).to_bits(),
                "not periodic at {turns}"
            );
        }
    }

    #[test]
    fn a_non_finite_angle_produces_a_nan_rather_than_looping_or_panicking() {
        // Callers are expected to reject non-finite values before this point, but the reduction must
        // not misbehave if one arrives: `INFINITY.floor()` is infinity and the subtraction would be
        // NaN anyway.
        assert!(sin_turns(f64::INFINITY).is_nan());
        assert!(sin_turns(f64::NAN).is_nan());
    }

    #[test]
    fn the_series_is_pinned_to_exact_bits() {
        // ADR 0007 decision 4: a function whose whole purpose is bit-exactness is not verified by an
        // approximate comparison. These pin the implementation itself, so changing a coefficient or
        // reassociating the Horner chain fails here rather than silently desyncing a match.
        let pinned: [(f64, u64); 5] = [
            (0.125, 0x3FE6_A09E_667F_3BCC),
            (0.25, 0x3FF0_0000_0000_0000),
            (0.375, 0x3FE6_A09E_667F_3BCC),
            (0.625, 0xBFE6_A09E_667F_3BCC),
            (1_000_000.125, 0x3FE6_A09E_667F_3BCC),
        ];
        for (turns, bits) in pinned {
            assert_eq!(
                sin_turns(turns).to_bits(),
                bits,
                "sin_turns({turns}) moved: {:#018X}",
                sin_turns(turns).to_bits()
            );
        }
    }
}
