//! The first verbs: spawn, move, stop — units that exist, go somewhere, and halt.
//!
//! This is M6's opening subsystem, deliberately smaller than the charter's full order set. A unit is
//! an owner, a template, a position, a speed, and a route it is walking.
//!
//! # Where a route comes from
//!
//! A move order asks [`Ground`] for one, at the tick the order lands, and the answer is a list of
//! waypoints. **When no ground grid is registered the route is the destination and nothing else**,
//! which is the straight line this subsystem walked before pathfinding existed — the honest
//! behaviour for a kernel that has been told nothing about terrain, and the reason a test can still
//! assemble a kernel out of `Units` alone.
//!
//! Grid or no grid, the *stepper* is the same, which is why [ADR
//! 3001](../../../docs/adr/3001-pathfinding.md) decision 6 could change what the move verb means
//! without changing what it encodes: walking a route is walking to a point, repeatedly.
//!
//! # And a route is re-decided when the ground under it changes
//!
//! Decision 7. A route is a plan made once against the grid as it stood, so the tick something
//! stamps that grid — a structure raised, a bridge lost — every unit whose remaining way crosses
//! the stamped cells plans again, in identifier order, before anybody takes a step. Without that
//! the pathfinder would be worse than the straight line it replaced: a unit would walk confidently
//! through a wall that went up in front of it, following a route that was correct when it was made.
//!
//! # The arithmetic is inside ADR 0007 without needing `cic-math` yet
//!
//! One tick of movement is a subtraction, a `sqrt`, a division, and a multiply-add per axis — every
//! one on the permitted list ([ADR 0007](../../../docs/adr/0007-simulation-arithmetic.md) decision
//! 3), so no transcendental is involved and the crate needs no trigonometry until something wants a
//! facing angle. The search that produced the route has no floating point in it at all. Units
//! deliberately store no heading: presentation derives one from the motion it sees, freely.
//!
//! # Commands are bytes, and this module owns their meaning
//!
//! The kernel treats command payloads as opaque; this is the layer that gives them one. The encoding
//! is fixed-width little-endian after a one-byte verb tag, built by [`spawn_command`],
//! [`move_command`] and [`stop_command`] so hosts never hand-assemble bytes.
//!
//! **A rejected command is counted, and the count is hashed.** An order for a unit you do not own, a
//! spawn of a template that cannot move, a payload that does not parse: each is ignored
//! deterministically — every machine sees the identical bytes and ignores identically — and the
//! counter makes the ignoring *visible*, so two machines that somehow disagree about a rejection
//! diverge on that tick instead of silently drifting apart.

use std::collections::BTreeMap;

use cic_assets::templates::{TemplateKind, TemplateSet};

use crate::command::PlayerId;
use crate::ground::{GROUND, Ground};
use crate::hash::StateHasher;
use crate::id::{IdAllocator, ObjectId};
use crate::subsystem::{Subsystem, TickContext};

/// The name the [`Units`] subsystem is registered and hashed under.
pub const UNITS: &str = "units";

/// The verb tag a spawn payload starts with.
const TAG_SPAWN: u8 = 1;
/// The verb tag a move payload starts with.
const TAG_MOVE: u8 = 2;
/// The verb tag a stop payload starts with.
const TAG_STOP: u8 = 3;

/// One unit.
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    /// The seat that owns it and may order it.
    pub owner: PlayerId,
    /// The template it was spawned from.
    pub template: String,
    /// Position on the ground plane, in metres. Elevation is presentation's to derive.
    pub position: [f64; 2],
    /// Speed in metres per second, captured from the template at spawn.
    pub speed: f64,
    /// The waypoints still to walk, in order. Empty when holding.
    ///
    /// A route rather than a point, because a unit going somewhere is going *by a way*: the
    /// pathfinder decides the way once, when the order lands, and the stepper below walks it. A
    /// straight-line order is the same thing with one waypoint in it, which is why the encoding of
    /// [`move_command`] did not have to change.
    pub route: Vec<[f64; 2]>,
}

impl Unit {
    /// Where the unit is heading right now — the next waypoint, not the final one.
    ///
    /// This is what presentation wants for a facing: a unit rounding a corner faces the corner, not
    /// the destination beyond it.
    #[must_use]
    pub fn heading_for(&self) -> Option<[f64; 2]> {
        self.route.first().copied()
    }

    /// Where the unit will end up, or `None` when it is holding.
    #[must_use]
    pub fn destination(&self) -> Option<[f64; 2]> {
        self.route.last().copied()
    }
}

/// The mobile roster: every unit, and the movement that advances them.
#[derive(Debug, Clone)]
pub struct Units {
    /// Unit-kind template speeds, captured at construction. What a spawn resolves against.
    speeds: BTreeMap<String, f64>,
    roster: BTreeMap<ObjectId, Unit>,
    /// Commands ignored so far — unknown templates, unowned units, unparseable bytes. Hashed, so a
    /// machine that ignored a different number of commands diverges on the tick it happened.
    rejected: u64,
    /// Routes recomputed so far because the ground under them was edited. Hashed for the same
    /// reason as [`Self::rejected`]: a repath changes where a unit goes without any order having
    /// been given, so two machines that repathed a different number of units have already parted
    /// company and this is the tick to say so.
    repathed: u64,
}

impl Units {
    /// A roster with no units, spawning from the given template set.
    #[must_use]
    pub fn new(templates: &TemplateSet) -> Self {
        let speeds = templates
            .templates
            .iter()
            .filter(|template| template.kind == TemplateKind::Unit)
            .filter_map(|template| {
                let speed = template.speed?;
                Some((template.id.clone(), f64::from(speed)))
            })
            .collect();
        Self {
            speeds,
            roster: BTreeMap::new(),
            rejected: 0,
            repathed: 0,
        }
    }

    /// The units, keyed by identifier.
    #[must_use]
    pub fn units(&self) -> &BTreeMap<ObjectId, Unit> {
        &self.roster
    }

    /// How many commands have been ignored.
    #[must_use]
    pub fn rejected(&self) -> u64 {
        self.rejected
    }

    /// How many routes have been recomputed because the ground beneath them was edited.
    #[must_use]
    pub fn repathed(&self) -> u64 {
        self.repathed
    }

    /// Recomputes the route of every unit whose remaining way crosses ground stamped this tick.
    ///
    /// [ADR 3001](../../../docs/adr/3001-pathfinding.md) decision 7. **In identifier order**,
    /// because a repath is a search and a search reads the same grid for everybody — the order
    /// decides nothing today and would decide everything the moment a repath could see another
    /// unit, and an unspecified order is exactly the nondeterminism this subsystem is written to
    /// avoid.
    ///
    /// A unit repaths to where it was already going, from where it now stands. A route that comes
    /// back empty is a unit that has been walled in, and it stops — which is the honest answer,
    /// and better than the alternative of keeping a route that leads through the new wall.
    ///
    /// Worst case one edit repaths the whole roster inside one tick. The record accepts that spike
    /// and names a deterministic repath queue as the mitigation if profiling ever objects.
    fn repath_over_edits(&mut self, ground: &Ground) {
        if ground.edits().is_empty() {
            return;
        }
        for unit in self.roster.values_mut() {
            let Some(destination) = unit.destination() else {
                continue;
            };
            if ground.route_crosses_an_edit(unit.position, &unit.route) {
                unit.route = ground.route(unit.position, destination);
                self.repathed += 1;
            }
        }
    }

    fn apply(
        &mut self,
        issuer: PlayerId,
        payload: &[u8],
        ids: &mut IdAllocator,
        ground: Option<&Ground>,
    ) {
        match decode(payload) {
            Some(Verb::Spawn { template, x, y }) => {
                let Some(&speed) = self.speeds.get(template) else {
                    self.rejected += 1;
                    return;
                };
                if !(x.is_finite() && y.is_finite()) {
                    self.rejected += 1;
                    return;
                }
                let id = ids.allocate();
                self.roster.insert(
                    id,
                    Unit {
                        owner: issuer,
                        template: template.to_owned(),
                        position: [x, y],
                        speed,
                        route: Vec::new(),
                    },
                );
            }
            Some(Verb::Move { unit, x, y }) => {
                if !(x.is_finite() && y.is_finite()) {
                    self.rejected += 1;
                    return;
                }
                match self.roster.get_mut(&unit) {
                    Some(subject) if subject.owner == issuer => {
                        // The route is decided here, on the tick the order lands, rather than being
                        // re-derived as the unit walks: a search whose result depended on when it ran
                        // would make the same order mean different things on two machines that
                        // agreed about everything else.
                        subject.route = match ground {
                            Some(ground) => ground.route(subject.position, [x, y]),
                            None => vec![[x, y]],
                        };
                    }
                    _ => self.rejected += 1,
                }
            }
            Some(Verb::Stop { unit }) => match self.roster.get_mut(&unit) {
                Some(subject) if subject.owner == issuer => {
                    subject.route.clear();
                }
                _ => self.rejected += 1,
            },
            None => self.rejected += 1,
        }
    }
}

impl Subsystem for Units {
    fn name(&self) -> &'static str {
        UNITS
    }

    fn tick(&mut self, context: &mut TickContext<'_>) {
        // The grid as it stands *this* tick, which is why `Ground` must be registered ahead of this
        // subsystem. `None` is a kernel that was never told about terrain, not a fault.
        let ground = context.peers.read::<Ground>(GROUND);

        // Commands first, in arrival order; then movement, in identifier order. A unit ordered this
        // tick therefore takes its first step this tick, which keeps "the tick a command lands" and
        // "the tick its effect starts" the same tick on every machine.
        let commands = context.commands;
        for command in commands {
            self.apply(command.player, &command.payload, context.ids, ground);
        }

        // Then the repath, after the orders rather than before them, so that "every route in the
        // roster was computed against this tick's grid" holds with no exceptions to remember. The
        // cost of putting it here is that a unit ordered on the same tick something was built
        // searches twice and gets the same answer; the cost of putting it first would be a rule
        // with an "unless" in it.
        if let Some(ground) = ground {
            self.repath_over_edits(ground);
        }

        let step = context.tick_seconds;
        for unit in self.roster.values_mut() {
            // One tick's travel, spent across as many legs as it reaches. Without the carry-over a
            // unit would forfeit the remainder of its step at every waypoint, so a route with
            // corners in it would be slower than the same distance in a straight line — and the
            // pathfinder would have made units worse at going places.
            //
            // The ground under the unit scales that travel, per ADR 3001 amendment B: a class ranks
            // routes and sets the pace on them, or grading the map would change which way a unit
            // went and not how long it took. Sampled once, from where the unit stands at the start
            // of the tick, rather than per leg — at thirty ticks a second a unit spends tens of
            // ticks inside one cell, so the difference is unobservable and the simpler rule is the
            // one that can be reasoned about.
            let pace = ground.map_or(1.0, |ground| ground.pace_at(unit.position));
            let mut reach = unit.speed * step * pace;
            while reach > 0.0 && !unit.route.is_empty() {
                let waypoint = unit.route[0];
                let dx = waypoint[0] - unit.position[0];
                let dy = waypoint[1] - unit.position[1];
                // Permitted operations only: the distance is a sqrt, the direction a division.
                // IEEE-754 requires both correctly rounded, so every machine takes the identical
                // step.
                let distance = (dx * dx + dy * dy).sqrt();
                if distance <= reach {
                    unit.position = waypoint;
                    unit.route.remove(0);
                    reach -= distance;
                } else {
                    unit.position[0] += dx / distance * reach;
                    unit.position[1] += dy / distance * reach;
                    reach = 0.0;
                }
            }
        }
    }

    fn write_state(&self, hasher: &mut StateHasher) {
        hasher.write_u64(self.roster.len() as u64);
        for (id, unit) in &self.roster {
            hasher.write_u64(id.0);
            hasher.write_bytes(&[unit.owner.0]);
            hasher.write_str(&unit.template);
            hasher.write_f64(unit.position[0]);
            hasher.write_f64(unit.position[1]);
            hasher.write_f64(unit.speed);
            // The whole remaining route, not just where it ends: two machines that agree about the
            // destination and disagree about the way there are diverged, and this is the tick to
            // say so rather than the tick the paths visibly part.
            hasher.write_u64(unit.route.len() as u64);
            for waypoint in &unit.route {
                hasher.write_f64(waypoint[0]);
                hasher.write_f64(waypoint[1]);
            }
        }
        hasher.write_u64(self.rejected);
        hasher.write_u64(self.repathed);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A decoded verb.
enum Verb<'payload> {
    Spawn {
        template: &'payload str,
        x: f64,
        y: f64,
    },
    Move {
        unit: ObjectId,
        x: f64,
        y: f64,
    },
    Stop {
        unit: ObjectId,
    },
}

/// Encodes a spawn: create a unit of `template` at `(x, y)` for the issuing seat.
///
/// # Panics
///
/// Panics if the template identifier is longer than 65535 bytes, which no validated template set
/// contains — the length prefix is a `u16` and a caller inventing identifiers that size is a
/// programming error rather than data.
#[must_use]
pub fn spawn_command(template: &str, x: f64, y: f64) -> Vec<u8> {
    let name = template.as_bytes();
    let mut payload = Vec::with_capacity(3 + name.len() + 16);
    payload.push(TAG_SPAWN);
    payload.extend(
        u16::try_from(name.len())
            .expect("a template identifier fits in a u16")
            .to_le_bytes(),
    );
    payload.extend(name);
    payload.extend(x.to_le_bytes());
    payload.extend(y.to_le_bytes());
    payload
}

/// Encodes a move: send `unit` toward `(x, y)`.
#[must_use]
pub fn move_command(unit: ObjectId, x: f64, y: f64) -> Vec<u8> {
    let mut payload = Vec::with_capacity(25);
    payload.push(TAG_MOVE);
    payload.extend(unit.0.to_le_bytes());
    payload.extend(x.to_le_bytes());
    payload.extend(y.to_le_bytes());
    payload
}

/// Encodes a stop: `unit` holds where it is.
#[must_use]
pub fn stop_command(unit: ObjectId) -> Vec<u8> {
    let mut payload = Vec::with_capacity(9);
    payload.push(TAG_STOP);
    payload.extend(unit.0.to_le_bytes());
    payload
}

/// Decodes a payload, or `None` for bytes that parse as no verb.
fn decode(payload: &[u8]) -> Option<Verb<'_>> {
    let (&tag, rest) = payload.split_first()?;
    match tag {
        TAG_SPAWN => {
            let (length, rest) = rest.split_at_checked(2)?;
            let length = usize::from(u16::from_le_bytes(length.try_into().ok()?));
            let (name, rest) = rest.split_at_checked(length)?;
            let template = std::str::from_utf8(name).ok()?;
            let (x, rest) = read_f64(rest)?;
            let (y, rest) = read_f64(rest)?;
            rest.is_empty().then_some(Verb::Spawn { template, x, y })
        }
        TAG_MOVE => {
            let (unit, rest) = read_u64(rest)?;
            let (x, rest) = read_f64(rest)?;
            let (y, rest) = read_f64(rest)?;
            rest.is_empty().then_some(Verb::Move {
                unit: ObjectId(unit),
                x,
                y,
            })
        }
        TAG_STOP => {
            let (unit, rest) = read_u64(rest)?;
            rest.is_empty().then_some(Verb::Stop {
                unit: ObjectId(unit),
            })
        }
        _ => None,
    }
}

fn read_u64(bytes: &[u8]) -> Option<(u64, &[u8])> {
    let (value, rest) = bytes.split_at_checked(8)?;
    Some((u64::from_le_bytes(value.try_into().ok()?), rest))
}

fn read_f64(bytes: &[u8]) -> Option<(f64, &[u8])> {
    let (value, rest) = read_u64(bytes)?;
    Some((f64::from_bits(value), rest))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use cic_assets::scenario::{ObjectPlacement, Position, Scenario, TerrainReference};
    use cic_assets::templates::{Footprint, Passage, Template, TemplateKind, TemplateSet};
    use cic_assets::terrain::Terrain;

    use super::{UNITS, Units, move_command, spawn_command, stop_command};
    use crate::activation::activate;
    use crate::command::{Command, PlayerId};
    use crate::ground::{GROUND, Ground, GroundRules, METALLED, PLAIN};
    use crate::id::ObjectId;
    use crate::kernel::{Kernel, KernelConfig, first_divergence};

    /// A quarter of a revolution, in the binary turns simulation state stores a heading as.
    const QUARTER_TURN: u32 = 1 << 30;

    fn templates() -> TemplateSet {
        TemplateSet {
            format_version: 1,
            templates: vec![
                Template {
                    id: "unit/rifleman".to_owned(),
                    kind: TemplateKind::Unit,
                    model: Some("models/rifleman.glb".to_owned()),
                    name: None,
                    speed: Some(6.0),
                    footprint: None,
                    passage: None,
                },
                // Four metres a second is a fixture choice, not a design one: at thirty ticks it
                // gives a step that divides none of the leg lengths below, which is what makes the
                // carry-over test able to fail. At the rifleman's six, every leg is a whole number
                // of steps and no remainder is ever left at a corner to be lost.
                Template {
                    id: "unit/sapper".to_owned(),
                    kind: TemplateKind::Unit,
                    model: Some("models/sapper.glb".to_owned()),
                    name: None,
                    speed: Some(4.0),
                    footprint: None,
                    passage: None,
                },
            ],
        }
    }

    /// The same set, plus a metalled road thirty-two cells long and one wide, and a wall.
    fn stamping_templates() -> TemplateSet {
        let mut set = templates();
        set.templates.push(Template {
            id: "structure/road".to_owned(),
            kind: TemplateKind::Structure,
            model: Some("models/road.glb".to_owned()),
            name: None,
            speed: None,
            footprint: None,
            passage: Some(Passage {
                cells: [32, 1],
                class: METALLED,
            }),
        });
        set.templates.push(Template {
            id: "structure/wall".to_owned(),
            kind: TemplateKind::Structure,
            model: Some("models/wall.glb".to_owned()),
            name: None,
            speed: None,
            footprint: Some(Footprint { cells: [1, 9] }),
            passage: None,
        });
        set.validate().expect("the fixture is a legal template set");
        set
    }

    /// Flat ground, `size` samples square, one metre a sample.
    fn flat_terrain(size: u32) -> Terrain {
        Terrain::new(
            size,
            size,
            1.0,
            1.0,
            vec![0u16; (size * size) as usize],
            Vec::new(),
        )
        .expect("a valid terrain")
    }

    /// One placement, on the ground plane and unrotated.
    fn placement(template: &str, x: f32, y: f32) -> ObjectPlacement {
        ObjectPlacement {
            template: template.to_owned(),
            position: Position { x, y, z: 0.0 },
            rotation: 0.0,
            scale: 1.0,
            owner: None,
        }
    }

    /// A kernel holding an activated scenario, then the grid, then the units.
    ///
    /// The registration order is the whole chain: `Forces` first so the grid reads the objects as
    /// they stand this tick, the grid second so the units path over it as it stands this tick.
    fn stamped_kernel(terrain: &Terrain, placements: Vec<ObjectPlacement>) -> Kernel {
        let templates = stamping_templates();
        let scenario = Scenario {
            format_version: 1,
            name: "stamps".to_owned(),
            description: String::new(),
            terrain: TerrainReference {
                path: "terrain/fixture.cict".to_owned(),
            },
            players: Vec::new(),
            objects: placements,
            waypoints: Vec::new(),
            scripts: Vec::new(),
        };
        let mut kernel = Kernel::new(KernelConfig {
            seed: 21,
            ticks_per_second: 30,
        });
        activate(&mut kernel, &scenario, &templates).expect("the fixture scenario activates");
        kernel.add_subsystem(Box::new(
            Ground::derive(terrain, GroundRules::default().with_sharp_corners())
                .with_templates(&templates),
        ));
        kernel.add_subsystem(Box::new(Units::new(&templates)));
        kernel
    }

    fn ground_of(kernel: &Kernel) -> &Ground {
        kernel
            .subsystem(GROUND)
            .and_then(|subsystem| subsystem.as_any().downcast_ref::<Ground>())
            .expect("ground registered")
    }

    /// A kernel with the units subsystem, at thirty ticks a second.
    fn kernel() -> Kernel {
        let mut kernel = Kernel::new(KernelConfig {
            seed: 21,
            ticks_per_second: 30,
        });
        kernel.add_subsystem(Box::new(Units::new(&templates())));
        kernel
    }

    fn command(tick: u64, player: u8, payload: Vec<u8>) -> Command {
        Command {
            tick,
            player: PlayerId(player),
            payload,
        }
    }

    fn units(kernel: &Kernel) -> &Units {
        kernel
            .subsystem(UNITS)
            .and_then(|subsystem| subsystem.as_any().downcast_ref::<Units>())
            .expect("units registered")
    }

    #[test]
    fn a_spawn_creates_a_unit_owned_by_its_issuer() {
        let mut kernel = kernel();
        kernel
            .advance(&[command(0, 1, spawn_command("unit/rifleman", 40.0, 60.0))])
            .expect("advances");
        let roster = units(&kernel);
        assert_eq!(roster.units().len(), 1);
        let unit = &roster.units()[&ObjectId(1)];
        assert_eq!(unit.owner, PlayerId(1));
        assert_eq!(unit.position, [40.0, 60.0]);
        assert_eq!(unit.speed, 6.0);
        assert_eq!(unit.destination(), None);
        assert_eq!(roster.rejected(), 0);
    }

    #[test]
    fn an_unknown_template_and_garbage_bytes_are_counted_not_faults() {
        let mut kernel = kernel();
        kernel
            .advance(&[
                command(0, 0, spawn_command("unit/absent", 0.0, 0.0)),
                command(0, 0, vec![9, 9, 9]),
                command(0, 0, Vec::new()),
            ])
            .expect("a rejected command must not stop the tick");
        assert_eq!(units(&kernel).units().len(), 0);
        assert_eq!(units(&kernel).rejected(), 3);
    }

    #[test]
    fn a_unit_walks_to_its_target_and_arrives_exactly() {
        let mut kernel = kernel();
        kernel
            .advance(&[command(0, 0, spawn_command("unit/rifleman", 0.0, 0.0))])
            .expect("advances");
        kernel
            .advance(&[command(1, 0, move_command(ObjectId(1), 3.0, 4.0))])
            .expect("advances");

        // Five metres at six metres a second is twenty-five ticks at thirty a second; the move
        // command's own tick already stepped once.
        for tick in 2..=25 {
            kernel.advance(&[]).expect("advances");
            let unit = &units(&kernel).units()[&ObjectId(1)];
            if unit.destination().is_none() {
                assert!(tick >= 25, "arrived early, at tick {tick}");
            }
        }
        let unit = &units(&kernel).units()[&ObjectId(1)];
        assert_eq!(unit.position, [3.0, 4.0], "arrival snaps to the target");
        assert_eq!(unit.destination(), None, "arriving clears the destination");
    }

    #[test]
    fn a_diagonal_step_is_no_faster_than_an_axial_one() {
        let mut kernel = kernel();
        kernel
            .advance(&[
                command(0, 0, spawn_command("unit/rifleman", 0.0, 0.0)),
                command(0, 0, spawn_command("unit/rifleman", 10.0, 0.0)),
            ])
            .expect("advances");
        kernel
            .advance(&[
                command(1, 0, move_command(ObjectId(1), 300.0, 400.0)),
                command(1, 0, move_command(ObjectId(2), 510.0, 0.0)),
            ])
            .expect("advances");

        let roster = units(&kernel);
        let diagonal = &roster.units()[&ObjectId(1)];
        let axial = &roster.units()[&ObjectId(2)];
        let moved = |unit: &super::Unit, from: [f64; 2]| {
            let dx = unit.position[0] - from[0];
            let dy = unit.position[1] - from[1];
            (dx * dx + dy * dy).sqrt()
        };
        let step = 6.0 / 30.0;
        assert!((moved(diagonal, [0.0, 0.0]) - step).abs() < 1e-12);
        assert!((moved(axial, [10.0, 0.0]) - step).abs() < 1e-12);
    }

    #[test]
    fn another_seats_unit_ignores_your_orders() {
        let mut kernel = kernel();
        kernel
            .advance(&[command(0, 0, spawn_command("unit/rifleman", 5.0, 5.0))])
            .expect("advances");
        kernel
            .advance(&[
                command(1, 1, move_command(ObjectId(1), 100.0, 100.0)),
                command(1, 1, stop_command(ObjectId(1))),
            ])
            .expect("advances");
        let roster = units(&kernel);
        assert_eq!(roster.units()[&ObjectId(1)].position, [5.0, 5.0]);
        assert_eq!(roster.units()[&ObjectId(1)].destination(), None);
        assert_eq!(roster.rejected(), 2);
    }

    #[test]
    fn stop_halts_a_unit_mid_route() {
        let mut kernel = kernel();
        kernel
            .advance(&[command(0, 0, spawn_command("unit/rifleman", 0.0, 0.0))])
            .expect("advances");
        kernel
            .advance(&[command(1, 0, move_command(ObjectId(1), 100.0, 0.0))])
            .expect("advances");
        kernel
            .advance(&[command(2, 0, stop_command(ObjectId(1)))])
            .expect("advances");
        let after_stop = units(&kernel).units()[&ObjectId(1)].position;
        kernel.advance(&[]).expect("advances");
        let later = units(&kernel).units()[&ObjectId(1)].position;
        assert_eq!(after_stop, later, "a stopped unit stays stopped");
        assert!(after_stop[0] > 0.0, "it had moved before the stop");
    }

    /// Nine samples square, flat, with a wall of raised samples down column four that stops two
    /// rows short of the far edge — so there is exactly one way around, along the bottom.
    fn walled_terrain() -> cic_assets::terrain::Terrain {
        let size = 9;
        let mut elevations = vec![0u16; (size * size) as usize];
        for y in 0..size - 2 {
            elevations[(y * size + 4) as usize] = 50;
        }
        cic_assets::terrain::Terrain::new(size, size, 1.0, 1.0, elevations, Vec::new())
            .expect("a valid terrain")
    }

    /// The same kernel, with the grid registered *ahead* of the units that read it.
    fn kernel_on(terrain: &cic_assets::terrain::Terrain) -> Kernel {
        let mut kernel = Kernel::new(KernelConfig {
            seed: 21,
            ticks_per_second: 30,
        });
        kernel.add_subsystem(Box::new(Ground::derive(
            terrain,
            crate::ground::GroundRules {
                maximum_grade: 1.0,
                water_level: None,
                ..crate::ground::GroundRules::default()
            }
            .with_sharp_corners(),
        )));
        kernel.add_subsystem(Box::new(Units::new(&templates())));
        kernel
    }

    #[test]
    fn a_unit_ordered_across_a_wall_walks_around_it() {
        let terrain = walled_terrain();
        let ground = Ground::derive(
            &terrain,
            crate::ground::GroundRules {
                maximum_grade: 1.0,
                water_level: None,
                ..crate::ground::GroundRules::default()
            }
            .with_sharp_corners(),
        );
        let mut kernel = kernel_on(&terrain);
        kernel
            .advance(&[command(0, 0, spawn_command("unit/rifleman", 2.5, 0.5))])
            .expect("advances");
        kernel
            .advance(&[command(1, 0, move_command(ObjectId(1), 6.5, 0.5))])
            .expect("advances");

        // The straight line between those two points crosses the wall, so a route that is one leg
        // long is a unit walking through it.
        let unit = &units(&kernel).units()[&ObjectId(1)];
        assert!(
            unit.route.len() > 1,
            "the order was answered with a straight line: {:?}",
            unit.route
        );

        for tick in 2..400 {
            kernel.advance(&[]).expect("advances");
            let unit = &units(&kernel).units()[&ObjectId(1)];
            let cell = ground.cell_at(unit.position).expect("on the grid");
            assert!(
                ground.passable(cell.0, cell.1),
                "tick {tick}: stood on impassable ground at {:?}",
                unit.position
            );
            if unit.destination().is_none() {
                break;
            }
        }
        let unit = &units(&kernel).units()[&ObjectId(1)];
        assert_eq!(unit.position, [6.5, 0.5], "it arrived, the long way");
    }

    #[test]
    fn a_road_carries_a_unit_faster_than_the_field_beside_it() {
        // ADR 3001 amendment B, and the whole chain that decision 4 opened: a template declares a
        // `passage`, a scenario places one, activation makes it an object, the grid stamps a
        // metalled lane where it stands, and a unit on that lane moves three times as fast as an
        // identical unit two dozen cells away on the ground it did not pave.
        //
        // This test used to vary what the *whole map's* open ground derived to — one map paved and
        // one not — because nothing could stamp a road. That version could not tell a road from a
        // renumbered ladder, which is the difference the entire mechanic is about.
        let terrain = flat_terrain(33);
        let mut kernel = stamped_kernel(&terrain, vec![placement("structure/road", 15.5, 1.5)]);

        assert_eq!(
            ground_of(&kernel).class(0, 1),
            PLAIN,
            "the stamp lands on the first tick, not at registration — the grid reads its peer, and \
             a peer is only readable inside a tick"
        );
        kernel
            .advance(&[
                command(0, 0, spawn_command("unit/rifleman", 0.5, 1.5)),
                command(0, 0, spawn_command("unit/rifleman", 0.5, 30.5)),
            ])
            .expect("advances");

        let ground = ground_of(&kernel);
        assert_eq!(
            ground.class(0, 1),
            METALLED,
            "the road is where the test put it"
        );
        assert_eq!(
            ground.class(31, 1),
            METALLED,
            "and it runs the width of the map"
        );
        assert_eq!(
            ground.class(0, 30),
            PLAIN,
            "the field beside it is ordinary ground"
        );
        // The road is the scenario's first placement, so activation gave it `ObjectId(1)` and the
        // two spawns that follow take the next two.
        kernel
            .advance(&[
                command(1, 0, move_command(ObjectId(2), 31.5, 1.5)),
                command(1, 0, move_command(ObjectId(3), 31.5, 30.5)),
            ])
            .expect("advances");
        for _ in 2..10 {
            kernel.advance(&[]).expect("advances");
        }

        let roster = units(&kernel);
        let road = roster.units()[&ObjectId(2)].position[0] - 0.5;
        let field = roster.units()[&ObjectId(3)].position[0] - 0.5;
        assert!(field > 0.0, "the unit in the field did not move at all");
        assert!(
            roster.units()[&ObjectId(3)].position[1] == 30.5,
            "the field unit strayed off its row, so it is not measuring the field"
        );
        assert!(
            (road - field * 3.0).abs() < 1e-9,
            "{road} metres of road against {field} of field is not the three-to-one the ladder sets"
        );
    }

    #[test]
    fn only_the_units_the_edit_touched_repath_and_only_on_that_tick() {
        // ADR 3001 decision 7 through the kernel, which is what makes it a statement about
        // `Units::tick` rather than about a method nothing calls. The stamp lands on the tick the
        // scenario activates; the unit whose route runs along the stamped row plans again, the one
        // twenty-nine rows away does not, and the tick after there is no news for either.
        let terrain = flat_terrain(33);
        let mut kernel = stamped_kernel(&terrain, vec![placement("structure/road", 15.5, 1.5)]);
        kernel
            .advance(&[
                command(0, 0, spawn_command("unit/rifleman", 0.5, 1.5)),
                command(0, 0, spawn_command("unit/rifleman", 0.5, 30.5)),
                // The road took `ObjectId(1)` at activation; these two are the spawns above.
                command(0, 0, move_command(ObjectId(2), 31.5, 1.5)),
                command(0, 0, move_command(ObjectId(3), 31.5, 30.5)),
            ])
            .expect("advances");
        assert_eq!(
            units(&kernel).repathed(),
            1,
            "exactly the unit standing on the new road should have replanned"
        );

        kernel.advance(&[]).expect("advances");
        assert_eq!(
            units(&kernel).repathed(),
            1,
            "an edit is news for one tick, or every unit replans for ever"
        );
    }

    #[test]
    fn a_wall_stamped_across_a_route_sends_the_unit_around_it() {
        // The behaviour decision 7 exists for. Nothing constructs anything yet, so the only way to
        // put a wall across a route already in progress is to hand the grid the object a
        // construction subsystem will hand it — which is exactly what `Ground::tick` does with the
        // `Forces` snapshot, one call further in.
        let terrain = flat_terrain(17);
        let templates = stamping_templates();
        let mut ground = Ground::derive(&terrain, GroundRules::default().with_sharp_corners())
            .with_templates(&templates);
        let mut roster = Units::new(&templates);
        let mut ids = crate::id::IdAllocator::new();

        let start = [0.5, 8.5];
        roster.apply(
            PlayerId(0),
            &spawn_command("unit/rifleman", start[0], start[1]),
            &mut ids,
            Some(&ground),
        );
        roster.apply(
            PlayerId(0),
            &move_command(ObjectId(1), 15.5, 8.5),
            &mut ids,
            Some(&ground),
        );
        assert_eq!(
            roster.units()[&ObjectId(1)].route,
            vec![[15.5, 8.5]],
            "over open ground the plan is a straight line, which is what the wall has to spoil"
        );

        ground.reconcile(
            &std::iter::once((
                ObjectId(9),
                crate::activation::Placed {
                    owner: None,
                    template: "structure/wall".to_owned(),
                    position: [8.5, 8.5, 0.0],
                    rotation: 0,
                    scale: 1.0,
                },
            ))
            .collect(),
        );
        assert!(!ground.passable(8, 8), "the wall is where the test put it");
        assert!(ground.passable(8, 3), "and it has an end to walk round");

        roster.repath_over_edits(&ground);
        assert_eq!(roster.repathed(), 1);

        let unit = &roster.units()[&ObjectId(1)];
        assert_eq!(
            unit.destination(),
            Some([15.5, 8.5]),
            "a repath goes where the unit was already going"
        );
        assert!(
            unit.route.len() > 1,
            "the unit is still planning to walk through the wall: {:?}",
            unit.route
        );
        let mut previous = start;
        for waypoint in &unit.route {
            assert!(
                ground.walkable_between(previous, *waypoint),
                "leg {previous:?} -> {waypoint:?} crosses the wall"
            );
            previous = *waypoint;
        }
    }

    #[test]
    fn a_unit_cut_off_by_a_stamp_gives_up_the_far_side() {
        // The other half of the rule, where the repath changes the *destination* rather than the
        // way to it. A wall goes up right across the map between a unit and where it was going, so
        // the best it can now do is the near side — which is what an order to the far side of a
        // wall has meant since decision 5, arriving here by a different road.
        let terrain = flat_terrain(17);
        let templates = stamping_templates();
        let mut ground = Ground::derive(&terrain, GroundRules::default().with_sharp_corners())
            .with_templates(&templates);
        let mut roster = Units::new(&templates);
        let mut ids = crate::id::IdAllocator::new();
        roster.apply(
            PlayerId(0),
            &spawn_command("unit/rifleman", 8.5, 15.5),
            &mut ids,
            Some(&ground),
        );
        roster.apply(
            PlayerId(0),
            &move_command(ObjectId(1), 8.5, 0.5),
            &mut ids,
            Some(&ground),
        );
        assert_eq!(
            roster.units()[&ObjectId(1)].destination(),
            Some([8.5, 0.5]),
            "the plan starts out reaching the far side"
        );

        // Two nine-cell walls, each stood on end by a quarter turn, laid across the whole map.
        let wall = |x: f64| crate::activation::Placed {
            owner: None,
            template: "structure/wall".to_owned(),
            position: [x, 12.5, 0.0],
            rotation: QUARTER_TURN,
            scale: 1.0,
        };
        ground.reconcile(
            &[(ObjectId(9), wall(4.5)), (ObjectId(10), wall(11.5))]
                .into_iter()
                .collect(),
        );
        assert!(
            (0..16).all(|x| !ground.passable(x, 12)),
            "the fixture needs a wall with no way round it, or it cannot fail"
        );

        roster.repath_over_edits(&ground);
        let unit = &roster.units()[&ObjectId(1)];
        assert_eq!(roster.repathed(), 1);
        assert!(
            unit.destination().is_some_and(|end| end[1] > 12.0),
            "the unit is still planning to arrive on the far side of a wall it cannot cross: {:?}",
            unit.route
        );
    }

    #[test]
    fn a_corner_costs_a_unit_no_time() {
        // The carry-over property, stated as the thing it protects: a route arrives on the tick its
        // *length* says it should, so bending around an obstacle is not additionally penalised by
        // the stepper. Without spending the remainder of a step across the corner, each of this
        // route's turns would strand a fraction of a tick and arrival would be late.
        let terrain = walled_terrain();
        let ground = Ground::derive(
            &terrain,
            crate::ground::GroundRules {
                maximum_grade: 1.0,
                water_level: None,
                ..crate::ground::GroundRules::default()
            }
            .with_sharp_corners(),
        );
        let start = [2.5, 0.5];
        let route = ground.route(start, [6.5, 0.5]);
        assert!(
            route.len() >= 3,
            "the fixture needs corners to be about them"
        );
        let length: f64 = route
            .iter()
            .fold((0.0, start), |(total, from), waypoint| {
                let (dx, dy) = (waypoint[0] - from[0], waypoint[1] - from[1]);
                (total + (dx * dx + dy * dy).sqrt(), *waypoint)
            })
            .0;

        let mut kernel = kernel_on(&terrain);
        kernel
            .advance(&[command(
                0,
                0,
                spawn_command("unit/sapper", start[0], start[1]),
            )])
            .expect("advances");
        kernel
            .advance(&[command(1, 0, move_command(ObjectId(1), 6.5, 0.5))])
            .expect("advances");

        // The move command's own tick already took the first step, so the order lands on tick 1 and
        // the walking ticks are counted from there.
        let step = 4.0 / 30.0;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a route a few metres long over a fifth-metre step is a small whole number"
        )]
        let expected = (length / step).ceil() as u64;
        let mut walked = 1;
        for tick in 2..400 {
            if units(&kernel).units()[&ObjectId(1)].destination().is_none() {
                break;
            }
            kernel.advance(&[]).expect("advances");
            walked = tick;
        }
        assert_eq!(
            walked, expected,
            "route of {length} metres in {walked} ticks"
        );
    }

    #[test]
    fn the_verbs_replay_identically() {
        let script: Vec<Command> = vec![
            command(0, 0, spawn_command("unit/rifleman", 0.0, 0.0)),
            command(0, 1, spawn_command("unit/rifleman", 200.0, 200.0)),
            command(2, 0, move_command(ObjectId(1), 97.0, 33.0)),
            command(2, 1, move_command(ObjectId(2), 13.0, 180.0)),
            command(30, 0, stop_command(ObjectId(1))),
            command(31, 1, move_command(ObjectId(2), 0.0, 0.0)),
        ];
        let run = |mut kernel: Kernel| {
            (0..90)
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
        let ours = run(kernel());
        let theirs = run(kernel());
        assert_eq!(first_divergence(&ours, &theirs), None);
        assert_eq!(ours, theirs);
    }

    #[test]
    fn pathfound_routes_replay_identically() {
        // The same seal over the pathfinding half. Two seats sending units across the wall in
        // opposite directions, re-ordered mid-route so a second search runs from a position the
        // first route put them at — the case where an accumulated position feeding back into a
        // search would show up, if either half were not deterministic.
        let terrain = walled_terrain();
        let script: Vec<Command> = vec![
            command(0, 0, spawn_command("unit/rifleman", 1.5, 1.5)),
            command(0, 1, spawn_command("unit/sapper", 6.5, 1.5)),
            command(2, 0, move_command(ObjectId(1), 7.5, 2.5)),
            command(2, 1, move_command(ObjectId(2), 0.5, 3.5)),
            command(40, 0, move_command(ObjectId(1), 2.5, 6.5)),
            command(55, 1, stop_command(ObjectId(2))),
            command(56, 1, move_command(ObjectId(2), 7.5, 7.5)),
        ];
        let run = || {
            let mut kernel = kernel_on(&terrain);
            (0..120)
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

        // And the grid is inside the hash, so a machine that derived a different one would be
        // caught by this comparison rather than by the units that later walked it.
        assert!(
            ours[0].entries.iter().any(|entry| entry.name == GROUND),
            "the ground did not contribute a hash entry"
        );
    }
}
