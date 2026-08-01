//! The heightfield both integration suites path over, and the coefficients they read it under.
//!
//! Shared because two suites now want the same ground: `pathfinding.rs` walks units across it, and
//! `formation.rs` scores where a group would be told to stand on it. A second copy of a noise
//! function is a second thing that can drift, and a fixture that drifts between two suites is one
//! that quietly stops comparing them.

#![allow(dead_code)]

use cic_assets::terrain::Terrain;
use cic_sim::ground::GroundRules;

/// Samples per axis. Sixty-five samples at eight metres is a 512-metre map — a quarter of the
/// generated demo's, big enough to have separate regions and small enough to run in a unit test.
pub const SAMPLES: u32 = 65;

/// Metres between samples, matching the demo terrain's spacing.
pub const SPACING: f32 = 8.0;

/// A rough heightfield: two interpolated octaves of value noise, in integers throughout, so every
/// machine builds the identical terrain to compare against.
///
/// **Interpolated matters.** A lattice sampled without it puts a cliff on every lattice boundary,
/// which makes a map of sealed pockets rather than a landscape — the first version of this fixture
/// did exactly that and seven of twelve units had nowhere at all to walk. Hills that *rise* mean
/// most ground is connected and the steep parts are features on it.
pub fn rough_terrain() -> Terrain {
    let mut elevations = Vec::with_capacity((SAMPLES * SAMPLES) as usize);
    for y in 0..SAMPLES {
        for x in 0..SAMPLES {
            // Amplitudes are chosen against the grade the rules below allow: eight metres of rise
            // per eight-metre cell. The coarse octave climbs about ten per sample at its steepest
            // and the fine one about six, so slopes straddle the threshold instead of sitting
            // entirely on one side of it.
            let height = octave(x, y, 16, 160) + octave(x, y, 4, 24);
            elevations.push(u16::try_from(height).unwrap_or(u16::MAX));
        }
    }
    Terrain::new(SAMPLES, SAMPLES, SPACING, 1.0, elevations, Vec::new()).expect("a valid terrain")
}

/// One octave: a lattice of hashed values every `period` samples, bilinearly interpolated.
///
/// The interpolation is a weighted sum divided once, so it stays exact in integers and no rounding
/// choice has to be pinned.
pub fn octave(x: u32, y: u32, period: u32, amplitude: u32) -> u32 {
    let (cell_x, cell_y) = (x / period, y / period);
    let (fx, fy) = (x % period, y % period);
    let corner = |dx: u32, dy: u32| mix(cell_x + dx, cell_y + dy) % amplitude;
    let (top_left, top_right) = (corner(0, 0), corner(1, 0));
    let (bottom_left, bottom_right) = (corner(0, 1), corner(1, 1));
    let (gx, gy) = (period - fx, period - fy);
    (top_left * gx * gy + top_right * fx * gy + bottom_left * gx * fy + bottom_right * fx * fy)
        / (period * period)
}

/// A small integer avalanche. Not a random stream — nothing here reaches simulation state, and a
/// fixture that changed between runs would make a failure impossible to reproduce.
pub fn mix(x: u32, y: u32) -> u32 {
    let mut value = x.wrapping_mul(0x9E37_79B9) ^ y.wrapping_mul(0x85EB_CA6B);
    value ^= value >> 15;
    value = value.wrapping_mul(0xC2B2_AE35);
    value ^ (value >> 13)
}

pub fn rules() -> GroundRules {
    GroundRules {
        maximum_grade: 1.0,
        water_level: Some(40.0),
        // Eight metres a cell here, so the default corner radius is the shape a real map gets. The
        // invariant these tests assert -- never standing on impassable ground -- is exactly the one
        // a rounding pass could break, so it is deliberately left switched on.
        ..GroundRules::default()
    }
}
