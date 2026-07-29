//! ADR 0007 decision 8, applied to this crate.
//!
//! > A test enforces the restriction textually, because `cargo build` will not. It scans the
//! > simulation crate for the forbidden names and fails naming the file and the call.
//!
//! This crate *is* the permitted arithmetic, so the restriction binds it more directly than it binds
//! anything else: a platform transcendental slipping into `sin_turns` would hand every consumer —
//! the script VM today, the kernel next — a value the other side of the match may not reproduce. The
//! guard travels with the code it guards, so extracting the arithmetic into its own crate moved the
//! scan here with it.
//!
//! # This lives in `tests/` deliberately
//!
//! The scanner needs the forbidden names as literals, so a scanner living in `src/` would find itself.
//! Putting it here means it can name them plainly instead of assembling them from fragments to hide
//! from its own search.
//!
//! # And it only scans shipped code
//!
//! Everything above the first `#[cfg(test)]` in a file. A test module is entitled to call the
//! platform's `sin` as an **oracle** — ADR 0007 does exactly that with `libm`, and
//! `tests::the_polynomial_agrees_with_the_platform_to_the_last_bit_on_the_same_argument` in `lib.rs`
//! is the comparison that gives this crate's series any credibility. The rule is that an oracle may be
//! measured against and may not be shipped.

use std::fs;
use std::path::Path;

/// Everything ADR 0007 decision 3 forbids in simulation code.
///
/// `sqrt` is *not* here and that is the point of the list: IEEE-754 requires it to be correctly
/// rounded, so it is on the permitted side along with the four arithmetic operations. What is banned
/// is precisely what the standard only recommends.
const FORBIDDEN: [&str; 22] = [
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "atan2",
    "exp",
    "exp2",
    "ln",
    "log",
    "log2",
    "log10",
    "powf",
    "powi",
    "hypot",
    "cbrt",
    "sinh",
    "cosh",
    "tanh",
    "to_degrees",
    "to_radians",
];

/// Returns the offending `(line number, line)` pairs in shipped code.
fn offences(source: &str) -> Vec<(usize, String)> {
    // A test module may use an oracle; shipped code may not.
    let shipped = source.split("#[cfg(test)]").next().unwrap_or(source);

    let mut found = Vec::new();
    for (index, full_line) in shipped.lines().enumerate() {
        // A comment is not a call. Documentation here routinely writes `sin(0.25)` while explaining
        // the very rule this enforces, and flagging that would train people to ignore the test --
        // which is the only way a tripwire actually fails.
        let line = full_line.split("//").next().unwrap_or(full_line);
        for name in FORBIDDEN {
            // A call rather than a mention: the name must be reached through `.` or `::` and be
            // immediately followed by its argument list. Without the trailing parenthesis this would
            // fire on `.expect(`, and without the leading separator it would fire on `sin_turns`.
            for prefix in [".", "::"] {
                if line.contains(&format!("{prefix}{name}(")) {
                    found.push((index + 1, full_line.trim().to_owned()));
                }
            }
        }
    }
    found
}

#[test]
fn no_shipped_code_calls_a_platform_transcendental() {
    let source_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let entries = fs::read_dir(&source_directory).expect("the crate has a src directory");

    let mut scanned = 0;
    let mut failures = Vec::new();
    for entry in entries {
        let path = entry.expect("a readable directory entry").path();
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        scanned += 1;
        let source = fs::read_to_string(&path).expect("a readable source file");
        let name = path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        for (line, text) in offences(&source) {
            failures.push(format!("{name}:{line}: {text}"));
        }
    }

    assert!(scanned >= 1, "no source files were scanned");
    assert!(
        failures.is_empty(),
        "ADR 0007 forbids a platform transcendental in simulation code, and these are calls to one:\n{}",
        failures.join("\n")
    );
}

#[test]
fn the_scanner_would_actually_catch_one() {
    // A test that cannot fail for the reason it claims is worse than no test. This proves the
    // mechanism, so the assertion above means something.
    let caught = offences("fn heading(x: f64) -> f64 {\n    x.sin()\n}\n");
    assert_eq!(caught.len(), 1, "a bare `.sin()` should have been caught");
    assert_eq!(caught[0].0, 2);

    assert_eq!(
        offences("fn f(x: f64) -> f64 { x.powf(2.0) }").len(),
        1,
        "`powf` should have been caught"
    );
    assert_eq!(
        offences("let y = f64::atan2(a, b);").len(),
        1,
        "an associated-function call should have been caught"
    );
}

#[test]
fn the_scanner_does_not_fire_on_what_is_permitted() {
    // The permitted set, and the near-misses that a looser pattern would flag. `sqrt` is permitted
    // because IEEE-754 requires it to be correctly rounded; `sin_turns` is this crate's own.
    for permitted in [
        "//! A quarter turn is `sin(0.25)` in prose, and prose is not a call.",
        "/// Delegates to `f64::atan2` -- no it does not, this is prose.",
        "let root = value.sqrt();",
        "let whole = value.floor();",
        "let size = value.abs().max(other).min(limit);",
        "let angle = sin_turns(turns) + cos_turns(turns);",
        "let clip = reader.read_exact(4).expect(\"header\");",
        "self.log.push(text.to_owned());",
        "const STANDARD: [(&str, u8); 1] = [(\"log\", 1)];",
        "let rounded = value.round().trunc();",
    ] {
        assert!(
            offences(permitted).is_empty(),
            "false positive on: {permitted}"
        );
    }
}

#[test]
fn a_test_module_may_use_an_oracle() {
    // ADR 0007 makes `libm` an oracle rather than an implementation, and this crate compares its
    // series against the platform's `sin` for the same reason. The scan stops at the test module so
    // that stays possible, and this pins that it does.
    let source = "fn ours(x: f64) -> f64 { polynomial(x) }\n\
                  #[cfg(test)]\n\
                  mod tests {\n    let expected = radians.sin();\n}\n";
    assert!(offences(source).is_empty());
}
