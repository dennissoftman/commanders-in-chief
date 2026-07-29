//! Reports how far two adapters' reference images differ, scene by scene.
//!
//! ```bash
//! cargo run -p cic-render --example compare_adapters -- <dir-a> <dir-b>
//! ```
//!
//! # Why this exists
//!
//! A reference set generated on the CI runner cannot be eyeballed against the one generated locally by
//! opening two windows side by side — the differences that matter are a few eight-bit steps in a gradient,
//! and the differences that do not matter look identical. Trusting a runner-generated set means measuring
//! each scene's delta against the same scene on the other adapter and confirming it is the size a
//! rasteriser difference should be, not the size a bug is.
//!
//! It reads only the committed images, so it needs no GPU and runs anywhere.

use std::error::Error;
use std::path::Path;

use cic_render::regression::{self, Tolerance};

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    let (Some(left), Some(right)) = (arguments.next(), arguments.next()) else {
        eprintln!("usage: compare_adapters <dir-a> <dir-b>");
        return Ok(());
    };
    let (left, right) = (Path::new(&left), Path::new(&right));

    let mut names: Vec<String> = std::fs::read_dir(left)?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .filter(|name| {
            Path::new(name)
                .extension()
                .is_some_and(|kind| kind == "png")
        })
        .map(|name| name.to_string_lossy().into_owned())
        .collect();
    names.sort();

    println!(
        "{:<28} {:>10} {:>10} {:>8}",
        "scene", "mean", "worst", "differing"
    );
    for name in names {
        let other = right.join(&name);
        if !other.exists() {
            println!("{name:<28} {:>10}", "absent");
            continue;
        }
        let (width, height, first) = regression::decode_png(&std::fs::read(left.join(&name))?)?;
        let (other_width, other_height, second) = regression::decode_png(&std::fs::read(other)?)?;
        if (width, height) != (other_width, other_height) {
            println!("{name:<28} {width}x{height} against {other_width}x{other_height}");
            continue;
        }
        let comparison =
            regression::compare_rgba(&first, &second, width, height, Tolerance::SAME_ADAPTER);
        println!(
            "{name:<28} {:>10.4} {:>10} {:>8.4}  {}",
            comparison.mean,
            comparison.peak,
            comparison.differing,
            if comparison.passes() {
                "within tolerance"
            } else {
                "OUTSIDE"
            }
        );
    }
    Ok(())
}
