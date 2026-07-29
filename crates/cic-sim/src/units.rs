//! The first verbs: spawn, move, stop — units that exist, go somewhere, and halt.
//!
//! This is M6's opening subsystem, deliberately smaller than the charter's full order set. A unit is
//! an owner, a template, a position, a speed, and possibly a destination; movement is a straight
//! line at the template's speed, because pathfinding is its own milestone line and a straight line
//! is what proves the command-to-motion pipe deterministically end to end.
//!
//! # The arithmetic is inside ADR 0007 without needing `cic-math` yet
//!
//! One tick of movement is a subtraction, a `sqrt`, a division, and a multiply-add per axis — every
//! one on the permitted list ([ADR 0007](../../../docs/adr/0007-simulation-arithmetic.md) decision
//! 3), so no transcendental is involved and the crate needs no trigonometry until something wants a
//! facing angle. Units deliberately store no heading: presentation derives one from the motion it
//! sees, freely.
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
    /// Where it is going, or `None` when holding.
    pub target: Option<[f64; 2]>,
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

    fn apply(&mut self, issuer: PlayerId, payload: &[u8], ids: &mut IdAllocator) {
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
                        target: None,
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
                        subject.target = Some([x, y]);
                    }
                    _ => self.rejected += 1,
                }
            }
            Some(Verb::Stop { unit }) => match self.roster.get_mut(&unit) {
                Some(subject) if subject.owner == issuer => {
                    subject.target = None;
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
        // Commands first, in arrival order; then movement, in identifier order. A unit ordered this
        // tick therefore takes its first step this tick, which keeps "the tick a command lands" and
        // "the tick its effect starts" the same tick on every machine.
        let commands = context.commands;
        for command in commands {
            self.apply(command.player, &command.payload, context.ids);
        }

        let step = context.tick_seconds;
        for unit in self.roster.values_mut() {
            let Some(target) = unit.target else { continue };
            let dx = target[0] - unit.position[0];
            let dy = target[1] - unit.position[1];
            // Permitted operations only: the distance is a sqrt, the direction a division. IEEE-754
            // requires both correctly rounded, so every machine takes the identical step.
            let distance = (dx * dx + dy * dy).sqrt();
            let reach = unit.speed * step;
            if distance <= reach {
                unit.position = target;
                unit.target = None;
            } else {
                unit.position[0] += dx / distance * reach;
                unit.position[1] += dy / distance * reach;
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
            match unit.target {
                Some(target) => {
                    hasher.write_bytes(&[1]);
                    hasher.write_f64(target[0]);
                    hasher.write_f64(target[1]);
                }
                None => hasher.write_bytes(&[0]),
            }
        }
        hasher.write_u64(self.rejected);
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

    use cic_assets::templates::{Template, TemplateKind, TemplateSet};

    use super::{UNITS, Units, move_command, spawn_command, stop_command};
    use crate::command::{Command, PlayerId};
    use crate::id::ObjectId;
    use crate::kernel::{Kernel, KernelConfig, first_divergence};

    fn templates() -> TemplateSet {
        TemplateSet {
            format_version: 1,
            templates: vec![Template {
                id: "unit/rifleman".to_owned(),
                kind: TemplateKind::Unit,
                model: Some("models/rifleman.glb".to_owned()),
                name: None,
                speed: Some(6.0),
            }],
        }
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
        assert_eq!(unit.target, None);
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
            if unit.target.is_none() {
                assert!(tick >= 25, "arrived early, at tick {tick}");
            }
        }
        let unit = &units(&kernel).units()[&ObjectId(1)];
        assert_eq!(unit.position, [3.0, 4.0], "arrival snaps to the target");
        assert_eq!(unit.target, None, "arriving clears the destination");
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
        assert_eq!(roster.units()[&ObjectId(1)].target, None);
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
}
