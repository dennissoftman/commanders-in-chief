//! Reference-image checking, shared by every rendering test binary.
//!
//! Deliberately exactly one public function rather than a set of helpers. Each integration test target
//! compiles this module separately, so anything not used by *all* of them would warn as dead code, and
//! CI denies warnings — a `#[allow]` covering that up would also hide a helper that had genuinely
//! stopped being used.
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
    let information = context.adapter_info();
    let slug = regression::adapter_slug(information.backend, &information.name);
    let directory = reference_root().join(&slug);
    let path = directory.join(name);
    let updating = std::env::var_os(UPDATE_VARIABLE).is_some();

    if updating || !path.exists() {
        std::fs::create_dir_all(&directory).expect("create the reference directory");
        std::fs::write(&path, capture.png().expect("encode capture")).expect("write reference");
        assert!(
            updating,
            "no reference existed for {name} on {slug}, so one was written to {}. \
             Open it, confirm it is what the renderer should produce, and commit it.",
            path.display()
        );
        eprintln!("updated reference {}", path.display());
        return;
    }

    let reference = std::fs::read(&path).expect("read the reference");
    let comparison = regression::compare(capture, &reference, Tolerance::SAME_ADAPTER)
        .expect("compare against the reference");
    if comparison.passes() {
        return;
    }

    let actual = output_root().join(format!("FAILED-{name}"));
    let difference = output_root().join(format!("FAILED-diff-{name}"));
    write(&actual, &capture.png().expect("encode capture"));
    match regression::difference_png(capture, &reference) {
        Ok(image) => write(&difference, &image),
        Err(error) => eprintln!("could not build a difference image: {error}"),
    }

    panic!(
        "{name} no longer matches its reference on {slug}: {comparison}\n  \
         reference: {}\n  capture:   {}\n  difference: {}\n  \
         If the change is intended, review those images and re-run with {UPDATE_VARIABLE}=1.",
        path.display(),
        actual.display(),
        difference.display()
    );
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
