//! Where a group of units stands when it gets where it was sent.
//!
//! [ADR 3003](../../../docs/adr/3003-formation-movement.md) is this module's specification. One
//! sentence of it decides everything here: **the formation is the one the group is already in.**
//! Slots are the members' own offsets from the group's centre, carried to the destination — so a
//! line arrives as a line, a wedge as a wedge, and a huddle as a huddle. Nothing is imposed and
//! nothing is invented.
//!
//! # Free, and not random
//!
//! Those are two separate requirements and they are met by two separate properties.
//!
//! *Free* is the translation: there is no box, no wedge table and no lattice to snap to, so the
//! shape a player arranged their army into survives being ordered somewhere.
//!
//! *Not random* is the **identity assignment**: member `i` goes to slot `i`, always. Nothing is
//! matched, scored or shuffled. On open ground every member's displacement is the same vector, so
//! the group translates rigidly and no two units ever cross paths — which is the property that
//! makes a group move read as one order rather than as `n` of them.
//!
//! # Two things the translation cannot do on its own, and what is done about them
//!
//! A group that sets out standing on top of itself would arrive standing on top of itself, and a
//! slot can land in a lake. So the translated slots are **opened out** until no two overlap, by the
//! same radius-aware half-overlap push local avoidance uses — a huddle becomes a rosette and a
//! spread-out group is left alone, because there is nothing to open. And a slot the ground refuses
//! is **re-placed on ground it does not**, widest member first, so the roomy cells go to the object
//! that needs them and the narrow ones fill in around it. That last ordering is the whole of
//! "big units placed efficiently": choosing by size rather than by identifier is what stops a
//! transporter being handed the one-cell gap while a rifleman takes the clearing.
//!
//! # Arithmetic
//!
//! Subtraction, addition, multiplication, division and `sqrt` — [ADR
//! 0007](../../../docs/adr/0007-simulation-arithmetic.md)'s permitted set, no transcendental. The
//! eight fan directions are a table rather than a circle walked with `sin` and `cos`, which is the
//! same choice the rest of this crate makes wherever an angle would otherwise appear.

use crate::ground::Ground;

/// One member of a group being ordered somewhere: where it stands, and how much room it needs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Member {
    /// Where it stands now, on the ground plane.
    pub position: [f64; 2],
    /// How much room to keep around it, in metres.
    pub radius: f64,
}

/// How many times the slots are opened out before the result is accepted as good enough.
///
/// A bound on a loop rather than a knob on the game. The loop stops as soon as nothing overlaps,
/// which is two or three rounds for the huddles that need any; this is the guard against a
/// pathological group — sixty-four units spawned on one point, say — turning one order into a long
/// tick. It is not on `AvoidanceRules` for that reason: nothing about a *match* should want a
/// different value, and a coefficient nobody would tune is a coefficient nobody should carry.
const SPREAD_ROUNDS: u32 = 24;

/// How much further apart than strictly necessary a crowded pair is pushed, as a multiple of the
/// room it needs.
///
/// Aiming at exactly "not overlapping" converges asymptotically: every round takes a share of a
/// shrinking overlap, and the last sliver takes for ever — the first version of this loop left five
/// units 3.99 metres apart where they needed 4.00, and would have needed hundreds of rounds to
/// close it. Aiming a little past the line means the loop reaches its own exit condition instead.
/// It reads better too: ranks with a hand's breadth between them look arranged, and ranks packed
/// shoulder to shoulder look spilled.
const SPREAD_SLACK: f64 = 1.05;

/// How far from a refused slot to look for ground that will take it, in cells.
///
/// Also a bound rather than a knob. Six cells is 48 metres on the demo's grid — far enough to walk
/// a slot out of a lake or off a cliff, and near enough that a member re-placed this way is still
/// recognisably part of the formation rather than off on its own.
const SETTLE_RINGS: u32 = 6;

/// Eight directions to open coincident slots along.
///
/// Two members standing in exactly the same place have no line between them to push along, so the
/// direction has to be chosen. Choosing it from a table indexed by *which pair* means a stack of
/// members opens into a rosette rather than a line, and it stays a decision rather than a random
/// number. Written out rather than walked with trigonometry, which this crate does not have.
const FAN: [[f64; 2]; 8] = [
    [1.0, 0.0],
    [
        core::f64::consts::FRAC_1_SQRT_2,
        core::f64::consts::FRAC_1_SQRT_2,
    ],
    [0.0, 1.0],
    [
        -core::f64::consts::FRAC_1_SQRT_2,
        core::f64::consts::FRAC_1_SQRT_2,
    ],
    [-1.0, 0.0],
    [
        -core::f64::consts::FRAC_1_SQRT_2,
        -core::f64::consts::FRAC_1_SQRT_2,
    ],
    [0.0, -1.0],
    [
        core::f64::consts::FRAC_1_SQRT_2,
        -core::f64::consts::FRAC_1_SQRT_2,
    ],
];

/// Where each member of a group should stand when it arrives at `destination`.
///
/// Returned in the order the members were given, because the assignment is the identity: member
/// `i` gets slot `i`. See the module documentation for why that is the whole of "not random".
///
/// `facing` is the direction the player dragged, if they dragged one — see [`turn_from`] for what
/// it does and, in [ADR 3003](../../../docs/adr/3003-formation-movement.md)'s **amendment A**, why
/// it is the only thing in here that ever rotates anything. `None` is a plain click, and a plain
/// click never rearranges a formation.
///
/// `ground` is optional for the same reason it is everywhere else in this crate: a kernel that was
/// never told about terrain has no ground to refuse a slot, and the answer is the translation
/// alone.
#[must_use]
pub fn slots(
    members: &[Member],
    destination: [f64; 2],
    facing: Option<[f64; 2]>,
    ground: Option<&Ground>,
) -> Vec<[f64; 2]> {
    if members.is_empty() {
        return Vec::new();
    }

    // The shape the group is already in, carried to where it was sent. Counted by adding rather
    // than by casting a length, which keeps the whole of this exact for any group a `u16` count
    // can name.
    let (mut sum, mut count) = ([0.0_f64; 2], 0.0_f64);
    for member in members {
        sum[0] += member.position[0];
        sum[1] += member.position[1];
        count += 1.0;
    }
    let centre = [sum[0] / count, sum[1] / count];
    // The identity unless the player asked for something else, which is the whole of "a click does
    // not rearrange your formation".
    let turn = facing.map_or([1.0, 0.0], |facing| {
        turn_from(
            [destination[0] - centre[0], destination[1] - centre[1]],
            facing,
        )
    });
    let mut slots: Vec<[f64; 2]> = members
        .iter()
        .map(|member| {
            let offset = [
                member.position[0] - centre[0],
                member.position[1] - centre[1],
            ];
            [
                destination[0] + offset[0] * turn[0] - offset[1] * turn[1],
                destination[1] + offset[0] * turn[1] + offset[1] * turn[0],
            ]
        })
        .collect();

    if spread(&mut slots, members) {
        // Only when something actually moved: opening a group out does not have to keep its centre
        // exactly, and a group that needed no opening must come through the translation untouched.
        recentre(&mut slots, destination);
    }
    if let Some(ground) = ground {
        settle(&mut slots, members, ground);
    }
    slots
}

/// The rotation that carries `from` onto `to`, as a `(cos, sin)` pair.
///
/// # Why there is no trigonometry in here
///
/// A facing is a *direction*, and the player gave it as one — the vector between the two points
/// they dragged. Turning that into an angle and back again would need `atan2` on one side and a
/// sine and cosine on the other, all three of them things [ADR
/// 0007](../../../docs/adr/0007-simulation-arithmetic.md) decision 3 refuses, to arrive at exactly
/// the number that was already in hand. The rotation from one unit vector to another *is* their
/// complex quotient: with both normalised, `to × conj(from)` is the `(cos, sin)` of the difference,
/// and applying it is four multiplies. Two square roots and two divisions, and nothing that a
/// platform library gets to have an opinion about.
///
/// A direction of no length has no rotation, and the answer is the identity — which is what makes a
/// group already standing on its destination, or one dragged nowhere, come through untouched.
fn turn_from(from: [f64; 2], to: [f64; 2]) -> [f64; 2] {
    let (Some(from), Some(to)) = (unit(from), unit(to)) else {
        return [1.0, 0.0];
    };
    [
        to[0] * from[0] + to[1] * from[1],
        to[1] * from[0] - to[0] * from[1],
    ]
}

/// A direction of length one, or `None` when there is no direction to be had.
fn unit(vector: [f64; 2]) -> Option<[f64; 2]> {
    let span = (vector[0] * vector[0] + vector[1] * vector[1]).sqrt();
    (span > 0.0 && span.is_finite()).then(|| [vector[0] / span, vector[1] / span])
}

/// Opens the slots out until no two of them overlap, or until the rounds run out. Answers whether
/// anything moved.
///
/// The correction per pair is local avoidance's: half the overlap each, along the line between
/// them. What is different is that each slot takes the **average** of what its neighbours ask for
/// rather than the sum, and that is not a detail — summing overshoots. A slot in the middle of a
/// huddle hears "move a whole overlap away" from four neighbours at once, moves four overlaps, and
/// the next round hears the same from the other side; the group oscillates outward instead of
/// settling. Averaging is the standard answer and it converges in two or three rounds on the cases
/// that need any.
fn spread(slots: &mut [[f64; 2]], members: &[Member]) -> bool {
    let mut moved = false;
    for _ in 0..SPREAD_ROUNDS {
        let mut pushes = vec![([0.0_f64; 2], 0.0_f64); slots.len()];
        let mut crowded = false;
        for first in 0..slots.len() {
            for second in first + 1..slots.len() {
                let room = members[first].radius + members[second].radius;
                let (dx, dy) = (
                    slots[second][0] - slots[first][0],
                    slots[second][1] - slots[first][1],
                );
                let gap = dx * dx + dy * dy;
                if gap >= room * room {
                    continue;
                }
                crowded = true;
                let span = gap.sqrt();
                let away = if span > 0.0 {
                    [dx / span, dy / span]
                } else {
                    // Coincident, so the direction comes from which pair this is rather than from
                    // a line that does not exist. Different pairs take different spokes, which is
                    // what turns a stack into a rosette instead of a queue.
                    FAN[(first * 3 + second) % FAN.len()]
                };
                let share = (room * SPREAD_SLACK - span) * 0.5;
                pushes[first].0[0] -= away[0] * share;
                pushes[first].0[1] -= away[1] * share;
                pushes[first].1 += 1.0;
                pushes[second].0[0] += away[0] * share;
                pushes[second].0[1] += away[1] * share;
                pushes[second].1 += 1.0;
            }
        }
        if !crowded {
            return moved;
        }
        moved = true;
        for (slot, (push, neighbours)) in slots.iter_mut().zip(pushes) {
            if neighbours > 0.0 {
                slot[0] += push[0] / neighbours;
                slot[1] += push[1] / neighbours;
            }
        }
    }
    moved
}

/// Slides every slot by the same vector so the group's centre sits on `destination` again.
///
/// Opening a huddle out moves each slot by the average of what its neighbours wanted, and averages
/// over different neighbour counts do not cancel — so the group can drift a little off the point it
/// was sent to. One subtraction each puts it back, and it makes "the group lands where it was sent"
/// true by construction rather than by an accident of symmetry.
fn recentre(slots: &mut [[f64; 2]], destination: [f64; 2]) {
    let (mut sum, mut count) = ([0.0_f64; 2], 0.0_f64);
    for slot in slots.iter() {
        sum[0] += slot[0];
        sum[1] += slot[1];
        count += 1.0;
    }
    let drift = [
        destination[0] - sum[0] / count,
        destination[1] - sum[1] / count,
    ];
    for slot in slots {
        slot[0] += drift[0];
        slot[1] += drift[1];
    }
}

/// Moves every slot the ground refuses onto ground it does not, widest member first.
///
/// **Widest first is the point.** The members whose slots are legal keep them, and the ones that
/// have to move choose from what is left in descending order of how much room they need — so the
/// widest object gets first refusal on the roomy ground and the narrow ones fit around it.
/// Choosing by identifier instead would hand the one clear cell to whichever unit happened to be
/// built first and leave the wide one squeezed against a cliff.
///
/// A re-placed slot must also be clear of every slot already settled. When nothing within reach
/// satisfies both, ground alone will do; when nothing satisfies even that, the slot is left where
/// it was and the router's own answer for an unreachable target takes over.
fn settle(slots: &mut [[f64; 2]], members: &[Member], ground: &Ground) {
    let mut refused: Vec<usize> = (0..slots.len())
        .filter(|index| !standable(ground, slots[*index]))
        .collect();
    // Widest first, and ties on the identifier order the caller supplied, so two members of one
    // size never depend on which way a sort happened to fall.
    refused.sort_by(|left, right| {
        members[*right]
            .radius
            .total_cmp(&members[*left].radius)
            .then(left.cmp(right))
    });

    for index in refused {
        let radius = members[index].radius;
        let wanted = slots[index];
        let roomy = nearest_cell(ground, wanted, |point| {
            standable(ground, point) && clear_of(point, radius, slots, members, index)
        });
        let picked =
            roomy.or_else(|| nearest_cell(ground, wanted, |point| standable(ground, point)));
        if let Some(point) = picked {
            slots[index] = point;
        }
    }
}

/// Whether a unit could stand at a world position at all.
fn standable(ground: &Ground, at: [f64; 2]) -> bool {
    ground
        .cell_at(at)
        .is_some_and(|(x, y)| ground.passable(x, y))
}

/// Whether a circle at `at` clears every slot but its own.
fn clear_of(
    at: [f64; 2],
    radius: f64,
    slots: &[[f64; 2]],
    members: &[Member],
    ignoring: usize,
) -> bool {
    slots.iter().enumerate().all(|(index, other)| {
        if index == ignoring {
            return true;
        }
        let room = radius + members[index].radius;
        let (dx, dy) = (other[0] - at[0], other[1] - at[1]);
        dx * dx + dy * dy >= room * room
    })
}

/// The nearest cell centre to `wanted` that `accept` allows, searched outward a ring at a time.
///
/// Nearest *within* a ring rather than first in it, because a ring is a square and its first cell
/// is a corner. Ties go to the earlier cell in a fixed row-major scan, which is a rule rather than
/// whatever the iterator happened to produce.
fn nearest_cell(
    ground: &Ground,
    wanted: [f64; 2],
    accept: impl Fn([f64; 2]) -> bool,
) -> Option<[f64; 2]> {
    let (cx, cy) = ground.cell_at(wanted)?;
    for ring in 0..=SETTLE_RINGS {
        let mut best: Option<([f64; 2], f64)> = None;
        for (x, y) in ring_cells(ground, cx, cy, ring) {
            let centre = ground.centre_of(x, y);
            if !accept(centre) {
                continue;
            }
            let (dx, dy) = (centre[0] - wanted[0], centre[1] - wanted[1]);
            let span = dx * dx + dy * dy;
            if best.is_none_or(|(_, held)| span < held) {
                best = Some((centre, span));
            }
        }
        if let Some((centre, _)) = best {
            return Some(centre);
        }
    }
    None
}

/// The cells exactly `ring` steps from `(cx, cy)` in Chebyshev distance, clipped to the grid.
fn ring_cells(
    ground: &Ground,
    cx: u32,
    cy: u32,
    ring: u32,
) -> impl Iterator<Item = (u32, u32)> + use<> {
    let low = (cx.saturating_sub(ring), cy.saturating_sub(ring));
    let high = (
        cx.saturating_add(ring)
            .min(ground.width().saturating_sub(1)),
        cy.saturating_add(ring)
            .min(ground.height().saturating_sub(1)),
    );
    (low.1..=high.1)
        .flat_map(move |y| (low.0..=high.0).map(move |x| (x, y)))
        .filter(move |(x, y)| ring == 0 || x.abs_diff(cx) == ring || y.abs_diff(cy) == ring)
}

#[cfg(test)]
mod tests {
    // The translation is exact — a slot is a destination plus a difference of two positions — so
    // equality is precisely the property several of these assert.
    #![allow(clippy::float_cmp)]

    use cic_assets::terrain::Terrain;

    use super::{Member, slots};
    use crate::ground::{Ground, GroundRules};

    fn member(x: f64, y: f64, radius: f64) -> Member {
        Member {
            position: [x, y],
            radius,
        }
    }

    /// The distance between two points.
    fn between(from: [f64; 2], to: [f64; 2]) -> f64 {
        let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
        (dx * dx + dy * dy).sqrt()
    }

    /// Whether any two members would be standing in each other at their slots.
    fn crowded(members: &[Member], slots: &[[f64; 2]]) -> bool {
        (0..slots.len()).any(|first| {
            (first + 1..slots.len()).any(|second| {
                between(slots[first], slots[second])
                    < members[first].radius + members[second].radius - 1e-9
            })
        })
    }

    /// Flat ground, `size` samples square, one metre a sample.
    fn flat(size: u32) -> Ground {
        let terrain = Terrain::new(
            size,
            size,
            1.0,
            1.0,
            vec![0u16; (size * size) as usize],
            Vec::new(),
        )
        .expect("a valid terrain");
        Ground::derive(&terrain, GroundRules::default())
    }

    #[test]
    fn one_unit_goes_exactly_where_it_was_sent() {
        // A group of one is a plain move, and has to stay one: the centroid is the unit's own
        // position, so the offset is zero and the slot is the destination untouched.
        let alone = [member(3.0, 4.0, 0.5)];
        assert_eq!(slots(&alone, [40.0, 60.0], None, None), vec![[40.0, 60.0]]);
    }

    #[test]
    fn a_group_arrives_in_the_shape_it_set_out_in() {
        // The whole idea, asserted as an exact translation. Three units in an L; every one of them
        // moves by the *same* vector, so every pairwise offset survives and nobody crosses anybody.
        // A formation generated from a template rather than carried would fail this whatever
        // template it used.
        let group = [
            member(10.0, 10.0, 0.5),
            member(14.0, 10.0, 0.5),
            member(10.0, 16.0, 0.5),
        ];
        let placed = slots(&group, [100.0, 200.0], None, None);

        let shift = [
            placed[0][0] - group[0].position[0],
            placed[0][1] - group[0].position[1],
        ];
        for (member, slot) in group.iter().zip(&placed) {
            assert_eq!(slot[0] - member.position[0], shift[0]);
            assert_eq!(slot[1] - member.position[1], shift[1]);
        }
    }

    #[test]
    fn the_group_lands_centred_on_where_it_was_sent() {
        let group = [
            member(10.0, 10.0, 0.5),
            member(14.0, 10.0, 0.5),
            member(10.0, 16.0, 0.5),
        ];
        let placed = slots(&group, [100.0, 200.0], None, None);
        let centre = placed.iter().fold([0.0, 0.0], |sum, slot| {
            [sum[0] + slot[0] / 3.0, sum[1] + slot[1] / 3.0]
        });
        assert!(between(centre, [100.0, 200.0]) < 1e-9, "{centre:?}");
    }

    #[test]
    fn a_group_that_set_out_in_a_stack_arrives_spread_out() {
        // The case the translation cannot answer on its own: five units in exactly one place have
        // no shape to carry, so they would arrive in exactly one place. They come out as a rosette
        // — a *structure*, not a scatter — and the group still lands where it was sent.
        let stacked: Vec<Member> = (0..5).map(|_| member(50.0, 50.0, 2.0)).collect();
        let placed = slots(&stacked, [10.0, 10.0], None, None);
        assert_eq!(placed.len(), 5);
        assert!(
            !crowded(&stacked, &placed),
            "five units still standing in each other: {placed:?}"
        );
        let centre = placed
            .iter()
            .fold([0.0, 0.0], |sum, slot| [sum[0] + slot[0], sum[1] + slot[1]]);
        let centre = [centre[0] / 5.0, centre[1] / 5.0];
        assert!(
            between(centre, [10.0, 10.0]) < 1e-9,
            "opening the stack moved the group off its destination: {centre:?}"
        );
        // A rosette rather than a queue: the slots use both axes, which a single fan direction
        // would not. Seeded from the first slot rather than from zero, which the first version of
        // this was and which made `spread_y` the distance from the origin — ten, whatever the slots
        // did. It passed against a deliberately one-axis fan, which is how it was found.
        let extent = |axis: usize| {
            let values = placed.iter().map(|slot| slot[axis]);
            let (low, high) = values.fold((f64::MAX, f64::MIN), |(low, high), value| {
                (low.min(value), high.max(value))
            });
            high - low
        };
        assert!(
            extent(0) > 1.0 && extent(1) > 1.0,
            "the stack opened into a line, {} by {}: {placed:?}",
            extent(0),
            extent(1)
        );
    }

    #[test]
    fn a_group_already_spread_out_is_not_opened_further() {
        // The complement, and what stops the spread pass being a slow drift outward: nothing
        // overlaps, so nothing moves, and the translation is exact.
        let group = [member(0.0, 0.0, 0.5), member(20.0, 0.0, 0.5)];
        let placed = slots(&group, [100.0, 100.0], None, None);
        assert_eq!(placed, vec![[90.0, 100.0], [110.0, 100.0]]);
    }

    #[test]
    fn a_slot_the_ground_refuses_is_moved_onto_ground_it_does_not() {
        // A wall of raised samples down column four makes cells three and four impassable. A group
        // translated across it has a slot inside the wall, and that slot has to end up somewhere a
        // unit could actually stand — near where it was meant to be, rather than anywhere.
        let size = 17;
        let mut elevations = vec![0u16; (size * size) as usize];
        for y in 0..size {
            elevations[(y * size + 8) as usize] = 50;
        }
        let terrain = Terrain::new(size, size, 1.0, 1.0, elevations, Vec::new()).expect("terrain");
        let ground = Ground::derive(&terrain, GroundRules::default());
        assert!(!ground.passable(8, 4), "the wall is where the test put it");

        let group = [member(0.0, 0.0, 0.4), member(2.0, 0.0, 0.4)];
        let placed = slots(&group, [8.5, 4.5], None, Some(&ground));
        for slot in &placed {
            let cell = ground.cell_at(*slot).expect("on the grid");
            assert!(
                ground.passable(cell.0, cell.1),
                "slot {slot:?} is inside the wall"
            );
        }
        assert!(
            placed.iter().all(|slot| between(*slot, [8.5, 4.5]) < 8.0),
            "a re-placed slot wandered off on its own: {placed:?}"
        );
    }

    /// Ground that is impassable everywhere except two small pockets.
    ///
    /// Built out of the water line rather than out of slopes, because a cell is under water when
    /// *all four* of its corners are — so raising one sample lifts exactly the four cells that
    /// touch it, and a pocket is one sample. Slopes would have made the same shape by arithmetic
    /// nobody could check by reading it.
    fn two_pockets() -> Ground {
        let size = 17;
        let mut elevations = vec![0u16; (size * size) as usize];
        for (sx, sy) in [(9u32, 5u32), (13, 5)] {
            elevations[(sy * size + sx) as usize] = 100;
        }
        let terrain =
            Terrain::new(size, size, 1.0, 1.0, elevations, Vec::new()).expect("a valid terrain");
        Ground::derive(
            &terrain,
            GroundRules {
                maximum_grade: 1000.0,
                water_level: Some(50.0),
                ..GroundRules::default()
            },
        )
    }

    #[test]
    fn the_two_pockets_fixture_is_the_shape_the_next_test_needs() {
        // Asserted before anything is asserted against it. The near pocket must be too small to
        // hold both members at once, or there is no contention and the test below cannot fail —
        // which is precisely what the first version of it did.
        let ground = two_pockets();
        let passable: Vec<(u32, u32)> = (0..ground.height())
            .flat_map(|y| (0..ground.width()).map(move |x| (x, y)))
            .filter(|(x, y)| ground.passable(*x, *y))
            .collect();
        assert_eq!(
            passable,
            vec![
                (8, 4),
                (9, 4),
                (12, 4),
                (13, 4),
                (8, 5),
                (9, 5),
                (12, 5),
                (13, 5)
            ],
            "two pockets of four cells, three cells apart"
        );
        // Every cell of the near pocket is within 1.7 metres of every other, which is the room a
        // 1.4-metre member and a 0.3-metre one need between them.
        for first in 0..4 {
            for second in first + 1..4 {
                let cells = [(8, 4), (9, 4), (8, 5), (9, 5)];
                let (here, there) = (
                    ground.centre_of(cells[first].0, cells[first].1),
                    ground.centre_of(cells[second].0, cells[second].1),
                );
                assert!(
                    between(here, there) < 1.7,
                    "the near pocket is too roomy to contend for"
                );
            }
        }
    }

    #[test]
    fn the_widest_member_gets_the_room() {
        // The requirement, in the one fixture that can show it. Both members are sent into water,
        // so both slots have to be re-placed; the near pocket is nearer to both of them and holds
        // only one of them, so somebody gets it and somebody walks on to the far one. Settling by
        // identifier would give it to whichever member the caller listed first. Settling by *size*
        // gives it to the one that has the harder time fitting anywhere — which is what "big units
        // placed efficiently" means.
        let ground = two_pockets();
        let wide = member(0.0, 0.0, 1.4);
        let narrow = member(0.0, 0.4, 0.3);
        let target = [10.5, 8.5];
        let near = ground.centre_of(9, 5);

        // The same two members, listed both ways round. The answer must depend on their sizes and
        // not on their places in the list.
        let straight = slots(&[wide, narrow], target, None, Some(&ground));
        let reversed = slots(&[narrow, wide], target, None, Some(&ground));

        assert!(
            between(straight[0], near) < between(straight[1], near),
            "listed first, the wide member did not take the near pocket: {straight:?}"
        );
        assert!(
            between(reversed[1], near) < between(reversed[0], near),
            "listed second, the wide member lost the near pocket to the narrow one: {reversed:?}"
        );

        for placed in [&straight, &reversed] {
            assert!(
                between(placed[0], placed[1]) >= wide.radius + narrow.radius,
                "the two members were settled standing in each other: {placed:?}"
            );
            for slot in placed {
                let cell = ground.cell_at(*slot).expect("on the grid");
                assert!(
                    ground.passable(cell.0, cell.1),
                    "slot {slot:?} is under water"
                );
            }
        }
    }

    #[test]
    fn a_click_never_turns_a_formation() {
        // The half of amendment A that matters most. Rearranging a shape nobody asked to have
        // rearranged is the thing this whole record exists to avoid, so it is asserted as bit
        // equality — and the destination is deliberately **off both axes**, because a group
        // travelling due east comes through an accidental turn-to-travel unchanged and the
        // assertion would then say nothing. That is exactly what the first version of it did, and
        // the sabotage pass is what found it.
        let line = [
            member(0.0, -4.0, 0.5),
            member(0.0, 0.0, 0.5),
            member(0.0, 4.0, 0.5),
        ];
        assert_eq!(
            slots(&line, [60.0, 80.0], None, None),
            vec![[60.0, 76.0], [60.0, 80.0], [60.0, 84.0]],
            "a click moved the formation as well as the group"
        );
    }

    #[test]
    fn a_drag_turns_the_formation_to_the_heading_it_asks_for() {
        // The other half. The group is travelling east and the line is abreast of that; dragged
        // north, the line has to come round with it and be abreast of *north* — a quarter turn,
        // exactly, and asserted as one.
        let line = [
            member(0.0, -4.0, 0.5),
            member(0.0, 0.0, 0.5),
            member(0.0, 4.0, 0.5),
        ];
        let dragged = slots(&line, [100.0, 0.0], Some([0.0, 7.0]), None);
        for (slot, expected) in dragged
            .iter()
            .zip([[104.0, 0.0], [100.0, 0.0], [96.0, 0.0]])
        {
            assert!(
                between(*slot, expected) < 1e-9,
                "dragged north the line should have turned: {dragged:?}"
            );
        }
    }

    #[test]
    fn dragging_the_way_the_group_is_already_going_changes_nothing() {
        // The identity case, and the one that says what a facing *means*: the shape is arranged for
        // a heading, so asking for the heading it is already arranged for is not an order to move
        // anybody. The drag is deliberately a different length from the trip, because only its
        // direction may count.
        let group = [member(3.0, 1.0, 0.5), member(-3.0, -1.0, 0.5)];
        let target = [50.0, 0.0];
        let clicked = slots(&group, target, None, None);
        let dragged = slots(&group, target, Some([9.0, 0.0]), None);
        for (click, drag) in clicked.iter().zip(&dragged) {
            assert!(
                between(*click, *drag) < 1e-9,
                "{clicked:?} against {dragged:?}"
            );
        }
    }

    #[test]
    fn a_drag_of_no_length_is_a_click() {
        let group = [member(0.0, -2.0, 0.5), member(0.0, 2.0, 0.5)];
        let target = [30.0, 30.0];
        let clicked = slots(&group, target, None, None);
        assert_eq!(slots(&group, target, Some([0.0, 0.0]), None), clicked);
        assert_eq!(slots(&group, target, Some([f64::NAN, 1.0]), None), clicked);
        // And a group already standing on its destination has no direction of travel to turn from,
        // so there is nothing a drag can mean and nothing it may do.
        let standing = [member(0.0, -2.0, 0.5), member(0.0, 2.0, 0.5)];
        assert_eq!(
            slots(&standing, [0.0, 0.0], Some([1.0, 1.0]), None),
            slots(&standing, [0.0, 0.0], None, None)
        );
    }

    #[test]
    fn turning_a_shape_does_not_change_it() {
        // A rotation is a rigid motion, so every distance inside the formation has to survive it.
        // This is what separates "the player chose an orientation" from "the game rearranged the
        // squad", and it is the property an implementation that reached for a shape table would
        // quietly lose.
        let group = [
            member(0.0, 0.0, 0.5),
            member(5.0, 2.0, 0.5),
            member(-3.0, 6.0, 0.5),
            member(1.0, -4.0, 0.5),
        ];
        let target = [200.0, 90.0];
        let clicked = slots(&group, target, None, None);
        let dragged = slots(&group, target, Some([-2.0, 5.0]), None);
        for first in 0..group.len() {
            for second in first + 1..group.len() {
                let before = between(clicked[first], clicked[second]);
                let after = between(dragged[first], dragged[second]);
                assert!(
                    (before - after).abs() < 1e-9,
                    "turning the shape stretched it: {before} became {after}"
                );
            }
        }
    }

    #[test]
    fn a_group_of_none_is_no_slots_rather_than_a_panic() {
        assert_eq!(slots(&[], [1.0, 2.0], None, None), Vec::<[f64; 2]>::new());
        assert_eq!(
            slots(&[], [1.0, 2.0], None, Some(&flat(9))),
            Vec::<[f64; 2]>::new()
        );
    }

    #[test]
    fn the_same_group_always_lands_the_same_way() {
        // No hashed container, no clock, no random stream: two calls with the same arguments have
        // to be bit-identical, because a slot is simulation state and reaches the tick hash through
        // the routes it produces.
        let group: Vec<Member> = (0..9)
            .map(|index| member(f64::from(index % 3), f64::from(index / 3), 1.0))
            .collect();
        let ground = flat(65);
        let once = slots(&group, [30.5, 30.5], None, Some(&ground));
        let twice = slots(&group, [30.5, 30.5], None, Some(&ground));
        assert_eq!(once, twice);
    }
}
