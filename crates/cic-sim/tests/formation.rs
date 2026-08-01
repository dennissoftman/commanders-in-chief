//! How much this formation method rearranges a squad, measured rather than argued about.
//!
//! The unit tests beside `cic_sim::formation` each pin one rule on ground built to show that rule.
//! This file asks the different question: **over a whole map, how often does the method touch a
//! shape the player arranged?** That is not a property an assertion can settle on one fixture,
//! because the answer depends on the ground — and it is the question that decides whether carrying
//! a formation is the right method at all, so it deserves a number and a place the number cannot
//! rot.
//!
//! # What is scored, and against what
//!
//! Every passable cell at a fixed stride is a destination, and an eight-unit squad is sent to each
//! one. Two things are counted:
//!
//! - **How many slots come through as a pure translation.** A slot that does is a member standing
//!   exactly where the shape says it should; a slot that does not is the game having rearranged
//!   somebody. This is the score for "does it leave my formation alone".
//! - **How many raw slots the ground refuses**, before any repair — the work the method makes for
//!   itself. This is the one where carrying a shape might plausibly *lose* to a generated one, so
//!   a compact box is laid over the same suite as a yardstick. It is not an implementation and
//!   nothing uses it; it is here to keep the comparison honest.
//!
//! Both numbers are asserted with room to move, so this is a regression harness for the method
//! rather than a transcription of today's output. A different method — a box, a wedge, a
//! matched assignment — can be scored against exactly the same suite, which is the point.

mod common;

use cic_sim::formation::{Member, slots};
use cic_sim::ground::Ground;

/// Every eleventh passable cell is a destination - three hundred and forty-one of them, which
/// is enough of the map that a percentage means something and few enough that the suite runs in
/// milliseconds.
const STRIDE: usize = 11;

/// Every `stride`-th passable cell, as a world position. The same spread the pathfinding suite
/// uses, for the same reason: a lattice of coordinates might fall entirely inside one basin.
fn destinations(ground: &Ground, stride: usize) -> Vec<[f64; 2]> {
    (0..ground.height())
        .flat_map(|y| (0..ground.width()).map(move |x| (x, y)))
        .filter(|(x, y)| ground.passable(*x, *y))
        .step_by(stride)
        .map(|(x, y)| ground.centre_of(x, y))
        .collect()
}

/// Eight units in two ranks of four, at four metres apart on an eight-metre grid.
///
/// A shape a player would recognise having made, rather than a random scatter — the question is
/// whether an *arrangement* survives being ordered somewhere, and a scatter has nothing to survive.
fn squad() -> Vec<Member> {
    (0..8)
        .map(|index| Member {
            position: [f64::from(index % 4) * 9.0, f64::from(index / 4) * 9.0],
            radius: 4.0,
        })
        .collect()
}

/// The group's centre.
fn centre(members: &[Member]) -> [f64; 2] {
    let (mut sum, mut count) = ([0.0_f64; 2], 0.0_f64);
    for member in members {
        sum[0] += member.position[0];
        sum[1] += member.position[1];
        count += 1.0;
    }
    [sum[0] / count, sum[1] / count]
}

fn between(from: [f64; 2], to: [f64; 2]) -> f64 {
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    (dx * dx + dy * dy).sqrt()
}

fn standable(ground: &Ground, at: [f64; 2]) -> bool {
    ground
        .cell_at(at)
        .is_some_and(|(x, y)| ground.passable(x, y))
}

/// A compact grid of slots centred on the destination — **a yardstick, not a feature.**
///
/// This is the method the record declined: throw the group's own shape away and generate one. It
/// exists here only so the number below has something to be compared against, because "the carried
/// shape needs repairing 6% of the time" means nothing until you know what the alternative needs.
fn box_slots(members: &[Member], destination: [f64; 2]) -> Vec<[f64; 2]> {
    let widest = members
        .iter()
        .map(|member| member.radius)
        .fold(0.0_f64, f64::max);
    let pitch = widest * 2.0 * 1.05;
    // The squarest grid that holds them: eight members make three columns.
    let columns = (1u32..=64)
        .find(|n| usize::try_from(n * n).unwrap_or(usize::MAX) >= members.len())
        .unwrap_or(1);
    let span = f64::from(columns - 1) * pitch * 0.5;
    (0..members.len())
        .map(|index| {
            let index = u32::try_from(index).unwrap_or(u32::MAX);
            [
                destination[0] + f64::from(index % columns) * pitch - span,
                destination[1] + f64::from(index / columns) * pitch - span,
            ]
        })
        .collect()
}

#[test]
fn the_suite_covers_ground_worth_scoring() {
    // The fixture asserts its own shape first. A suite of destinations that were all in one meadow
    // would score any method perfectly and settle nothing.
    let ground = Ground::derive(&common::rough_terrain(), common::rules());
    let targets = destinations(&ground, STRIDE);
    assert!(
        targets.len() > 80,
        "only {} destinations, which is not a map's worth",
        targets.len()
    );
    let refused = targets
        .iter()
        .flat_map(|target| box_slots(&squad(), *target))
        .filter(|slot| !standable(&ground, *slot))
        .count();
    assert!(
        refused > 40,
        "only {refused} slots of the yardstick land on bad ground, so this terrain is too open to \
         tell two methods apart"
    );
}

#[test]
fn a_squad_ordered_across_the_map_keeps_the_shape_it_was_put_in() {
    // The score for "does it leave my formation alone". A slot that comes through as an exact
    // translation is a member standing where the shape says; anything else is the game having
    // moved somebody the player did not.
    let ground = Ground::derive(&common::rough_terrain(), common::rules());
    let squad = squad();
    let origin = centre(&squad);
    let targets = destinations(&ground, STRIDE);

    let (mut carried, mut rearranged, mut worst, mut refused) =
        (0_usize, 0_usize, 0.0_f64, 0_usize);
    for target in &targets {
        let placed = slots(&squad, *target, None, Some(&ground));
        for (member, slot) in squad.iter().zip(&placed) {
            let ideal = [
                target[0] + member.position[0] - origin[0],
                target[1] + member.position[1] - origin[1],
            ];
            if !standable(&ground, ideal) {
                refused += 1;
            }
            let drift = between(*slot, ideal);
            if drift < 1e-9 {
                carried += 1;
            } else {
                rearranged += 1;
                worst = worst.max(drift);
            }
        }
    }

    // Measured at 2612 of 2728 — **95.7%** — with the furthest moved slot 35 metres from where the
    // shape put it. The bounds are looser than that on purpose: this is a harness for the method
    // rather than a transcription of one afternoon's output.
    let total = carried + rearranged;
    assert!(
        carried * 10 >= total * 9,
        "only {carried} of {total} slots came through untouched, so the method is rearranging \
         squads it was not asked to"
    );
    assert!(
        worst < 60.0,
        "a slot was moved {worst} metres from where the shape put it, which is not the same \
         formation any more"
    );

    // And the sharper claim, which is the one a player would notice: **the only slots that move
    // are the ones the ground refused.** Nothing is rearranged for tidiness, for packing, or
    // because an algorithm preferred a different arrangement. It holds for a squad that is already
    // spread out, which is what a formation is; a squad standing in a heap would also see the
    // opening-out pass, and that is the only other thing in the method that can move anybody.
    assert_eq!(
        rearranged, refused,
        "{rearranged} slots moved but only {refused} were on ground that refused them, so \
         something in the method is rearranging squads for reasons of its own"
    );
}

#[test]
fn carrying_a_shape_does_not_cost_much_more_repair_than_generating_one() {
    // The trade-off the method could plausibly lose on, so it is the one worth measuring. A carried
    // shape is as sprawling as the player left it and a generated box is as compact as it can be,
    // so the box should need *less* repair on rough ground. The question is how much less: if
    // carrying a shape cost several times the repair, keeping it would be paid for in units
    // shuffling around on arrival, and the record's choice would be the wrong one.
    let ground = Ground::derive(&common::rough_terrain(), common::rules());
    let squad = squad();
    let origin = centre(&squad);
    let targets = destinations(&ground, STRIDE);

    let mut carried_refused = 0_usize;
    let mut boxed_refused = 0_usize;
    let mut total = 0_usize;
    for target in &targets {
        for member in &squad {
            let translated = [
                target[0] + member.position[0] - origin[0],
                target[1] + member.position[1] - origin[1],
            ];
            if !standable(&ground, translated) {
                carried_refused += 1;
            }
            total += 1;
        }
        boxed_refused += box_slots(&squad, *target)
            .into_iter()
            .filter(|slot| !standable(&ground, *slot))
            .count();
    }

    assert!(total > 600, "the suite is too small to compare anything");
    // Measured at 116 refused against the box's 72, on 2728 slots — **4.3% against 2.6%** — so
    // carrying the shape costs about **1.6 times** the repair of the tightest packing there is, and
    // buys a formation that survives untouched 95.7% of the time. That is the trade this method
    // makes, in two numbers, and it is why the record chose as it did. Twice the box's repair is
    // where it would stop being worth it.
    assert!(
        carried_refused <= boxed_refused * 2,
        "carrying the shape refused {carried_refused} of {total} slots against the box's \
         {boxed_refused}, which is enough worse that keeping the shape is being paid for in repair"
    );
}

#[test]
fn every_slot_the_method_returns_is_one_a_unit_could_stand_on() {
    // The hard floor under both scores above. Whatever the method does to a shape, it may not hand
    // back a place nothing can stand — and on a map with this much water and cliff in it, that is
    // a claim the whole suite is needed to make.
    let ground = Ground::derive(&common::rough_terrain(), common::rules());
    let squad = squad();
    let mut stranded = Vec::new();
    for target in destinations(&ground, STRIDE) {
        for slot in slots(&squad, target, None, Some(&ground)) {
            if !standable(&ground, slot) {
                stranded.push((target, slot));
            }
        }
    }
    assert!(
        stranded.is_empty(),
        "{} slots landed on ground nothing can stand on, the first at {:?}",
        stranded.len(),
        stranded.first()
    );
}
