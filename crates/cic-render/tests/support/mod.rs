//! The shared GPU device and reference-image checking, for every rendering test binary.
//!
//! Two public functions, both used by all three targets. Each integration test target compiles this
//! module separately, so anything not used by *all* of them would warn as dead code, and CI denies
//! warnings — a `#[allow]` covering that up would also hide a helper that had genuinely stopped being
//! used.
//!
//! The comparison itself lives in `cic_render::regression`, which takes bytes rather than paths because
//! nothing above the resource layer opens a file. The file handling is here, in the tests, where it
//! belongs.

use std::path::{Path, PathBuf};

use cic_render::gpu::{Capture, GpuContext};
use cic_render::regression::{self, Tolerance};

/// Set this to rewrite every reference the run touches instead of comparing against it.
///
/// The only supported way to move a reference forward. A rendering change is either intended — in which
/// case the new images are reviewed and committed deliberately — or it is a regression, and there is no
/// third case that should quietly overwrite the evidence.
const UPDATE_VARIABLE: &str = "CIC_UPDATE_REFERENCES";

/// Set this where an adapter is *expected*, so its absence fails the run instead of skipping it.
///
/// A skipped rendering test and a passing one are the same colour, which is fine on a developer machine
/// with no GPU and actively misleading in CI. Without this, installing a software rasteriser on the
/// runner and forgetting to make it *usable* would leave every render test skipping and the job green —
/// indistinguishable from the state before the rasteriser was installed, and the harness would protect
/// nothing while appearing to. CI sets this, so a runner that loses its adapter fails loudly.
const REQUIRE_VARIABLE: &str = "CIC_REQUIRE_ADAPTER";

/// Builds the device a rendering test binary shares, or explains why there is none.
///
/// One implementation rather than three identical copies, because the requirement check below has to
/// hold for every target — a guard present in two binaries out of three is not a guard.
///
/// # Panics
///
/// Panics when no adapter can be had and [`REQUIRE_VARIABLE`] is set.
pub fn shared_context() -> Option<GpuContext> {
    match pollster::block_on(GpuContext::new()) {
        Ok(context) => {
            let information = context.adapter_info();
            // The adapter name and backend decide which reference set this run compares against, so
            // printing them is what makes a "no reference existed" failure diagnosable from a CI log
            // alone. `--nocapture` is not needed: a run that fails prints it.
            eprintln!(
                "adapter: {} ({:?}), reference set {}",
                information.name,
                information.backend,
                regression::adapter_slug(information.backend, &information.name)
            );
            Some(context)
        }
        Err(error) => {
            assert!(
                std::env::var_os(REQUIRE_VARIABLE).is_none(),
                "no usable GPU adapter ({error}), and {REQUIRE_VARIABLE} is set. \
                 Every rendering test would have skipped and the run would have passed \
                 without drawing anything. On a CI runner this means the software rasteriser \
                 is missing or the loader cannot see it."
            );
            eprintln!("skipping: no usable adapter ({error})");
            None
        }
    }
}

/// Compares a capture against its committed reference for the current adapter.
///
/// Panics with the measured difference when they disagree, having first written both the capture and an
/// amplified difference image next to the other test output — because the number says a regression
/// happened and only the images say what it was.
///
/// When no reference exists yet, one is written and the test **fails**. Passing instead would mean a
/// deleted or missing reference silently removed the coverage it was providing, which is the one
/// failure mode this whole mechanism exists to prevent.
pub fn check_reference(context: &GpuContext, name: &str, capture: &Capture) {
    check_references(context, &[(name, capture)]);
}

/// Compares several captures from one scene against their references, reporting on all of them.
///
/// The same check as [`check_reference`] and it exists for one reason: that one *stops at the first
/// problem*, which is right for a test with a single capture and wrong for a test with three. Two
/// consequences, and the second is the expensive one.
///
/// A regression that moved every kind of water reports one of them, so the reader fixes it, re-runs,
/// and is told about the next.
///
/// Worse, **bootstrapping a new adapter takes one run per capture.** A missing reference is written
/// and then fails the test, so the second and third captures in a test are never reached and never
/// written — and on a CI runner, where the reference set arrives by downloading an artefact, that is
/// a whole round trip each. The three water kinds cost two of those before this existed.
///
/// So every capture here is written or compared before anything panics, and the panic names all of
/// them at once.
pub fn check_references(context: &GpuContext, captures: &[(&str, &Capture)]) {
    let mut written = Vec::new();
    let mut failures = Vec::new();
    for (name, capture) in captures {
        match compare_reference(context, name, capture) {
            Outcome::Matched => {}
            Outcome::Written(path) => written.push(path),
            Outcome::Failed(report) => failures.push(report),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} captures no longer match their references:\n\n{}",
        failures.len(),
        captures.len(),
        failures.join("\n\n")
    );
    assert!(
        written.is_empty(),
        "no reference existed for {} of {} captures, so each was written:\n  {}\n\
         Open them, confirm they are what the renderer should produce, and commit them.",
        written.len(),
        captures.len(),
        written.join("\n  ")
    );
}

/// What comparing one capture against its reference produced.
enum Outcome {
    Matched,
    /// No reference existed; one was written to this path.
    Written(String),
    /// The capture and the reference disagree, described for a reader.
    Failed(String),
}

fn compare_reference(context: &GpuContext, name: &str, capture: &Capture) -> Outcome {
    let information = context.adapter_info();
    let slug = regression::adapter_slug(information.backend, &information.name);
    let directory = reference_root().join(&slug);
    let path = directory.join(name);
    let updating = std::env::var_os(UPDATE_VARIABLE).is_some();

    if updating || !path.exists() {
        std::fs::create_dir_all(&directory).expect("create the reference directory");
        std::fs::write(&path, capture.png().expect("encode capture")).expect("write reference");
        if !updating {
            return Outcome::Written(path.display().to_string());
        }
        eprintln!("updated reference {}", path.display());
        return Outcome::Matched;
    }

    let reference = std::fs::read(&path).expect("read the reference");
    let comparison = regression::compare(capture, &reference, Tolerance::SAME_ADAPTER)
        .expect("compare against the reference");
    if comparison.passes() {
        return Outcome::Matched;
    }

    let actual = output_root().join(format!("FAILED-{name}"));
    let difference = output_root().join(format!("FAILED-diff-{name}"));
    write(&actual, &capture.png().expect("encode capture"));
    match regression::difference_png(capture, &reference) {
        Ok(image) => write(&difference, &image),
        Err(error) => eprintln!("could not build a difference image: {error}"),
    }

    Outcome::Failed(format!(
        "{name} no longer matches its reference on {slug}: {comparison}\n  \
         reference: {}\n  capture:   {}\n  difference: {}\n  \
         If the change is intended, review those images and re-run with {UPDATE_VARIABLE}=1.",
        path.display(),
        actual.display(),
        difference.display()
    ))
}

/// Where committed references live, keyed by adapter beneath this.
fn reference_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("references")
}

/// Where a failing run leaves its evidence: the same place the captures already go.
fn output_root() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
}

fn write(path: &Path, bytes: &[u8]) {
    if let Err(error) = std::fs::write(path, bytes) {
        eprintln!("could not write {}: {error}", path.display());
    }
}
