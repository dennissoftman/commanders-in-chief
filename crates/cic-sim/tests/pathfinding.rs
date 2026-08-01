//! Pathfinding over ground that looks like a map, rather than over a fixture with one wall in it.
//!
//! The unit tests beside the implementation each isolate one rule — the corner rule, the water line,
//! the unreachable fallback — on terrain built to show exactly that rule and nothing else. This file
//! is the other half: a rough heightfield with hills, water and a scatter of unreachable pockets,
//! many units crossing it at once, and one invariant asserted on every tick of every run.
//!
//! **No unit ever stands on ground the grid calls impassable.** That is the property the whole
//! module exists to deliver, it holds regardless of which route the search picks, and it is the kind
//! of statement a screenshot of a unit halfway up a hill cannot make — the lesson the renderer's
//! half-pixel bug taught this project, applied before there is a bug to learn it from.

// Exact comparison is the question being asked here: a unit that never moved holds the bit pattern
// it was spawned with, because nothing wrote to it. A tolerance would turn "did not move" into "did
// not move much", which is a different and weaker claim.
#![allow(clippy::float_cmp)]

use cic_assets::templates::{Template, TemplateKind, TemplateSet};
use cic_assets::terrain::Terrain;
use cic_sim::command::{Command, PlayerId};
use cic_sim::ground::{Ground, GroundRules};
use cic_sim::kernel::{Kernel, KernelConfig, first_divergence};
use cic_sim::units::{UNITS, Units, move_command, spawn_command};

/// Samples per axis. Sixty-five samples at eight metres is a 512-metre map — a quarter of the
/// generated demo's, big enough to have separate regions and small enough to run in a unit test.
const SAMPLES: u32 = 65;

/// Metres between samples, matching the demo terrain's spacing.
const SPACING: f32 = 8.0;

/// A rough heightfield: two interpolated octaves of value noise, in integers throughout, so every
/// machine builds the identical terrain to compare against.
///
/// **Interpolated matters.** A lattice sampled without it puts a cliff on every lattice boundary,
/// which makes a map of sealed pockets rather than a landscape — the first version of this fixture
/// did exactly that and seven of twelve units had nowhere at all to walk. Hills that *rise* mean
/// most ground is connected and the steep parts are features on it.
fn rough_terrain() -> Terrain {
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
fn octave(x: u32, y: u32, period: u32, amplitude: u32) -> u32 {
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
fn mix(x: u32, y: u32) -> u32 {
    let mut value = x.wrapping_mul(0x9E37_79B9) ^ y.wrapping_mul(0x85EB_CA6B);
    value ^= value >> 15;
    value = value.wrapping_mul(0xC2B2_AE35);
    value ^ (value >> 13)
}

fn rules() -> GroundRules {
    GroundRules {
        maximum_grade: 1.0,
        water_level: Some(40.0),
    }
}

fn templates() -> TemplateSet {
    TemplateSet {
        format_version: 1,
        templates: vec![Template {
            id: "unit/scout".to_owned(),
            kind: TemplateKind::Unit,
            model: None,
            name: None,
            speed: Some(40.0),
        }],
    }
}

fn kernel(terrain: &Terrain) -> Kernel {
    let mut kernel = Kernel::new(KernelConfig {
        seed: 4,
        ticks_per_second: 30,
    });
    kernel.add_subsystem(Box::new(Ground::derive(terrain, rules())));
    kernel.add_subsystem(Box::new(Units::new(&templates())));
    kernel
}

fn units(kernel: &Kernel) -> &Units {
    kernel
        .subsystem(UNITS)
        .and_then(|subsystem| subsystem.as_any().downcast_ref::<Units>())
        .expect("units registered")
}

/// `wanted` passable cells spread evenly through the map, as world positions.
///
/// Spread by counting rather than by coordinate, so they land in whatever regions the terrain
/// actually has instead of on a lattice that might fall entirely inside one basin.
fn scattered_starts(ground: &Ground, wanted: usize) -> Vec<[f64; 2]> {
    let passable: Vec<(u32, u32)> = (0..ground.height())
        .flat_map(|y| (0..ground.width()).map(move |x| (x, y)))
        .filter(|(x, y)| ground.passable(*x, *y))
        .collect();
    let stride = (passable.len() / wanted).max(1);
    passable
        .iter()
        .step_by(stride)
        .take(wanted)
        .map(|(x, y)| ground.centre_of(*x, *y))
        .collect()
}

#[test]
fn the_fixture_is_ground_worth_pathing_over() {
    // A map with nothing blocked would satisfy every assertion below without exercising anything,
    // and a map with everything blocked would satisfy them by having nobody move. Both failure
    // modes are the same one — a fixture that cannot fail — so the fixture asserts its own shape
    // before anything is asserted against it.
    let ground = Ground::derive(&rough_terrain(), rules());
    let cells = (ground.width() * ground.height()) as usize;
    let blocked = (0..ground.height())
        .flat_map(|y| (0..ground.width()).map(move |x| (x, y)))
        .filter(|(x, y)| !ground.passable(*x, *y))
        .count();
    assert!(
        blocked * 20 > cells && blocked * 2 < cells,
        "{blocked} of {cells} cells blocked, which is not a map with obstacles on it"
    );
}

#[test]
fn no_unit_ever_stands_on_impassable_ground() {
    let terrain = rough_terrain();
    let ground = Ground::derive(&terrain, rules());
    let starts = scattered_starts(&ground, 12);
    assert_eq!(starts.len(), 12, "the fixture should offer twelve starts");

    let mut kernel = kernel(&terrain);
    let spawns: Vec<Command> = starts
        .iter()
        .enumerate()
        .map(|(index, start)| Command {
            tick: 0,
            player: PlayerId(u8::try_from(index % 2).expect("zero or one")),
            payload: spawn_command("unit/scout", start[0], start[1]),
        })
        .collect();
    kernel.advance(&spawns).expect("advances");

    // Each unit is sent to the start of the unit eight places along, so the orders cross the map in
    // several directions at once and some of them are certainly unreachable.
    let orders: Vec<Command> = (0..starts.len())
        .map(|index| {
            let target = starts[(index + 8) % starts.len()];
            Command {
                tick: 1,
                player: PlayerId(u8::try_from(index % 2).expect("zero or one")),
                payload: move_command(
                    cic_sim::id::ObjectId(index as u64 + 1),
                    target[0],
                    target[1],
                ),
            }
        })
        .collect();
    kernel.advance(&orders).expect("advances");
    assert_eq!(units(&kernel).rejected(), 0, "every order was accepted");

    let mut moved = 0;
    for tick in 2..600 {
        kernel.advance(&[]).expect("advances");
        for (id, unit) in units(&kernel).units() {
            let (x, y) = ground.cell_at(unit.position).expect("on the grid");
            assert!(
                ground.passable(x, y),
                "tick {tick}: unit {id:?} stood on impassable ground at {:?}",
                unit.position
            );
        }
    }
    for (start, unit) in starts.iter().zip(units(&kernel).units().values()) {
        if unit.position != *start {
            moved += 1;
        }
    }
    assert!(moved >= 10, "only {moved} of twelve units went anywhere");
}

#[test]
fn a_crowded_map_replays_identically() {
    let terrain = rough_terrain();
    let ground = Ground::derive(&terrain, rules());
    let starts = scattered_starts(&ground, 16);

    let script: Vec<Command> = starts
        .iter()
        .enumerate()
        .flat_map(|(index, start)| {
            let seat = PlayerId(u8::try_from(index % 4).expect("under four"));
            let id = cic_sim::id::ObjectId(index as u64 + 1);
            let target = starts[(index * 5 + 3) % starts.len()];
            [
                Command {
                    tick: 0,
                    player: seat,
                    payload: spawn_command("unit/scout", start[0], start[1]),
                },
                Command {
                    tick: (index as u64 % 7) + 1,
                    player: seat,
                    payload: move_command(id, target[0], target[1]),
                },
            ]
        })
        .collect();

    let run = || {
        let mut kernel = kernel(&terrain);
        (0..300)
            .map(|tick| {
                let this_tick: Vec<Command> = script
                    .iter()
                    .filter(|command| command.tick == tick)
                    .cloned()
                    .collect();
                kernel.advance(&this_tick).expect("advances")
            })
            .collect::<Vec<_>>()
    };
    let (ours, theirs) = (run(), run());
    assert_eq!(first_divergence(&ours, &theirs), None);
    assert_eq!(ours, theirs);
}
