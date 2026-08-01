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
use cic_sim::units::{
    AvoidanceRules, UNITS, Units, move_command, move_group_command, spawn_command,
};

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
        // Eight metres a cell here, so the default corner radius is the shape a real map gets. The
        // invariant these tests assert -- never standing on impassable ground -- is exactly the one
        // a rounding pass could break, so it is deliberately left switched on.
        ..GroundRules::default()
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
            // Four metres against eight-metre cells, so a scout is half a cell wide and a
            // crowd of them ordered to one point genuinely has to spread out.
            radius: Some(4.0),
            footprint: None,
            passage: None,
        }],
    }
}

fn kernel(terrain: &Terrain) -> Kernel {
    kernel_avoiding(terrain, AvoidanceRules::default())
}

/// The same kernel under chosen avoidance coefficients, so a test can run one crowd twice and
/// compare it against itself rather than against a number somebody wrote down.
fn kernel_avoiding(terrain: &Terrain, avoidance: AvoidanceRules) -> Kernel {
    let mut kernel = Kernel::new(KernelConfig {
        seed: 4,
        ticks_per_second: 30,
    });
    kernel.add_subsystem(Box::new(Ground::derive(terrain, rules())));
    kernel.add_subsystem(Box::new(Units::new(&templates()).with_rules(avoidance)));
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

/// Every passable cell at least `reach` cells inside the map, with how much impassable ground sits
/// within `reach` of it.
///
/// The two crowd tests below want opposite ends of this: one wants somewhere a crowd can spread out
/// freely, and the other wants somewhere it gets pressed against the terrain. Picking either by
/// hand would be a coordinate nobody could check, and picking "the first passable cell" would give
/// whichever the scan happened to meet.
fn musters(ground: &Ground, reach: u32) -> Vec<((u32, u32), usize)> {
    (reach..ground.height() - reach)
        .flat_map(|y| (reach..ground.width() - reach).map(move |x| (x, y)))
        // Gatherable: a crowd has to be able to stand on the spot itself and step off it.
        .filter(|(x, y)| {
            [(0i32, 0i32), (1, 0), (-1, 0), (0, 1), (0, -1)]
                .into_iter()
                .all(|(dx, dy)| {
                    ground.passable(x.wrapping_add_signed(dx), y.wrapping_add_signed(dy))
                })
        })
        .map(|(x, y)| {
            let blocked = (y - reach..=y + reach)
                .flat_map(|ny| (x - reach..=x + reach).map(move |nx| (nx, ny)))
                .filter(|(nx, ny)| !ground.passable(*nx, *ny))
                .count();
            ((x, y), blocked)
        })
        .collect()
}

/// How many pairs of units are standing in each other.
fn overlapping_pairs(kernel: &Kernel) -> usize {
    let bodies: Vec<([f64; 2], f64)> = units(kernel)
        .units()
        .values()
        .map(|unit| (unit.position, unit.radius))
        .collect();
    let mut crowded = 0;
    for (index, (here, near)) in bodies.iter().enumerate() {
        for (there, far) in bodies.iter().skip(index + 1) {
            let (dx, dy) = (there[0] - here[0], there[1] - here[1]);
            let room = near + far;
            // A hair of slack, because a pass that stops when the overlap is gone leaves the pair
            // exactly touching and floating-point exactly is not a thing to assert against.
            if dx * dx + dy * dy < room * room - 1e-6 {
                crowded += 1;
            }
        }
    }
    crowded
}

#[test]
fn a_crowd_ordered_to_one_point_stops_standing_in_itself() {
    // ADR 3001 decision 10 at scale: sixteen units sent to the *same* cell, mustering on the most
    // open ground the map has so that spreading out is the only thing being measured. The
    // comparison is the crowd against itself with avoidance switched off, which is what makes this
    // a statement about the mechanism rather than about a number somebody measured once.
    //
    // The impassable-ground assertion in here is a guard rather than a claim: nothing near this
    // muster point is close enough to be pushed into, and deleting the grid check was measured
    // leaving this test green. `a_crowd_pressed_against_the_terrain_is_never_pushed_over_it` is the
    // one that can fail on it.
    let terrain = rough_terrain();
    let ground = Ground::derive(&terrain, rules());
    let starts = scattered_starts(&ground, 16);
    let (open, clear) = musters(&ground, 3)
        .into_iter()
        .min_by_key(|(cell, blocked)| (*blocked, *cell))
        .expect("the fixture needs somewhere to muster");
    assert_eq!(clear, 0, "the open muster point is not open");
    let muster = ground.centre_of(open.0, open.1);

    let run = |avoidance| {
        let mut kernel = kernel_avoiding(&terrain, avoidance);
        let spawns: Vec<Command> = starts
            .iter()
            .map(|start| Command {
                tick: 0,
                player: PlayerId(0),
                payload: spawn_command("unit/scout", start[0], start[1]),
            })
            .collect();
        kernel.advance(&spawns).expect("advances");
        let orders: Vec<Command> = (0..starts.len())
            .map(|index| Command {
                tick: 1,
                player: PlayerId(0),
                payload: move_command(
                    cic_sim::id::ObjectId(index as u64 + 1),
                    muster[0],
                    muster[1],
                ),
            })
            .collect();
        kernel.advance(&orders).expect("advances");
        for tick in 2..400 {
            kernel.advance(&[]).expect("advances");
            for (id, unit) in units(&kernel).units() {
                let (x, y) = ground.cell_at(unit.position).expect("on the grid");
                assert!(
                    ground.passable(x, y),
                    "tick {tick}: unit {id:?} was pushed onto impassable ground at {:?}",
                    unit.position
                );
            }
        }
        overlapping_pairs(&kernel)
    };

    let stacked = run(AvoidanceRules { separation: 0.0 });
    let spread = run(AvoidanceRules::default());
    assert!(
        stacked > 40,
        "only {stacked} of 120 pairs stacked with avoidance off, so the fixture never crowded and          cannot show avoidance working"
    );
    assert!(
        spread * 4 < stacked,
        "{spread} pairs still standing in each other against {stacked} with avoidance off"
    );
}

#[test]
fn a_crowd_pressed_against_the_terrain_is_never_pushed_over_it() {
    // The other crowd, and the one the grid clamp exists for. The muster point is the passable cell
    // with the *most* impassable ground around it that a unit can still stand on and step off, so
    // sixteen eight-metre circles arriving at it have nowhere to spread but into the water and the
    // cliffs — which is precisely the pressure a push that did not consult the grid would relieve
    // by putting somebody in a lake.
    //
    // The open-ground crowd above cannot make this statement and was measured proving that it
    // cannot: with the grid check deleted it still passed, because nothing there ever pushed
    // anybody near anything.
    let terrain = rough_terrain();
    let ground = Ground::derive(&terrain, rules());
    let starts = scattered_starts(&ground, 16);
    let (tight, blocked) = musters(&ground, 3)
        .into_iter()
        .max_by_key(|(cell, blocked)| (*blocked, std::cmp::Reverse(*cell)))
        .expect("the fixture needs somewhere to muster");
    assert!(
        blocked > 12,
        "the tightest muster point has only {blocked} impassable cells around it, which is not a          crowd pressed against anything"
    );
    let muster = ground.centre_of(tight.0, tight.1);

    let mut kernel = kernel(&terrain);
    let spawns: Vec<Command> = starts
        .iter()
        .map(|start| Command {
            tick: 0,
            player: PlayerId(0),
            payload: spawn_command("unit/scout", start[0], start[1]),
        })
        .collect();
    kernel.advance(&spawns).expect("advances");
    let orders: Vec<Command> = (0..starts.len())
        .map(|index| Command {
            tick: 1,
            player: PlayerId(0),
            payload: move_command(
                cic_sim::id::ObjectId(index as u64 + 1),
                muster[0],
                muster[1],
            ),
        })
        .collect();
    kernel.advance(&orders).expect("advances");

    for tick in 2..400 {
        kernel.advance(&[]).expect("advances");
        for (id, unit) in units(&kernel).units() {
            let (x, y) = ground.cell_at(unit.position).expect("on the grid");
            assert!(
                ground.passable(x, y),
                "tick {tick}: unit {id:?} was pushed onto impassable ground at {:?}",
                unit.position
            );
        }
    }

    // And the crowd really did gather there, or the assertion above was about nothing.
    let gathered = units(&kernel)
        .units()
        .values()
        .filter(|unit| {
            let (dx, dy) = (unit.position[0] - muster[0], unit.position[1] - muster[1]);
            dx * dx + dy * dy < 40.0 * 40.0
        })
        .count();
    assert!(
        gathered >= 6,
        "only {gathered} of sixteen units reached the muster point, so nothing was pressed against          anything"
    );
}

#[test]
fn one_group_order_beats_sixteen_orders_to_the_same_point() {
    // [ADR 3003](../../../docs/adr/3003-formation-movement.md) against the case ADR 3001 decision
    // 10 named as its own limitation. Sixteen units sent to one cell by sixteen separate orders end
    // up jostling, because every one of them is walking at a point every other one is standing on;
    // sent as *one* order they are given sixteen places and simply stand in them.
    //
    // The comparison is the same crowd, the same map and the same tick budget, differing only in
    // whether the destination was one point or a formation — so the number below is about the
    // mechanism rather than about a tolerance somebody chose.
    let terrain = rough_terrain();
    let ground = Ground::derive(&terrain, rules());
    let starts = scattered_starts(&ground, 16);
    let (open, clear) = musters(&ground, 3)
        .into_iter()
        .min_by_key(|(cell, blocked)| (*blocked, *cell))
        .expect("the fixture needs somewhere to muster");
    assert_eq!(clear, 0, "the open muster point is not open");
    let muster = ground.centre_of(open.0, open.1);
    let group: Vec<cic_sim::id::ObjectId> = (0..starts.len())
        .map(|index| cic_sim::id::ObjectId(index as u64 + 1))
        .collect();

    let run = |together: bool| {
        let mut kernel = kernel(&terrain);
        let spawns: Vec<Command> = starts
            .iter()
            .map(|start| Command {
                tick: 0,
                player: PlayerId(0),
                payload: spawn_command("unit/scout", start[0], start[1]),
            })
            .collect();
        kernel.advance(&spawns).expect("advances");
        let orders: Vec<Command> = if together {
            vec![Command {
                tick: 1,
                player: PlayerId(0),
                payload: move_group_command(&group, muster[0], muster[1]),
            }]
        } else {
            group
                .iter()
                .map(|id| Command {
                    tick: 1,
                    player: PlayerId(0),
                    payload: move_command(*id, muster[0], muster[1]),
                })
                .collect()
        };
        kernel.advance(&orders).expect("advances");
        for tick in 2..400 {
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
        overlapping_pairs(&kernel)
    };

    let separately = run(false);
    let together = run(true);
    assert!(
        separately > 8,
        "only {separately} pairs jostled when sent separately, so the fixture cannot show a          formation helping"
    );
    assert!(
        together * 4 < separately,
        "{together} pairs still standing in each other after a group order, against {separately}          after sixteen separate ones"
    );
}
