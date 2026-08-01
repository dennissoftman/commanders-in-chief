//! Scenario activation: a map's declared players and placements become kernel state.
//!
//! This is where the kernel meets the asset formats. [`activate`] reads a validated
//! [`Scenario`] — who plays, on which team, starting where, and what is placed — resolves every
//! template and faction reference against the [`TemplateSet`], and constructs the [`Forces`]
//! subsystem from it, allocating every object's identifier from the kernel's own counter in
//! **authored order**. Authored order is the rule everywhere an order is needed here, for the
//! reason mount order and dispatch order use it too: it is explicit, visible in a diff, and cannot
//! differ between machines.
//!
//! # What activation is not
//!
//! No behaviour. A placed object has an owner, a template name, a pose — and no verbs, because verbs
//! are gameplay and gameplay is M6. `Forces::tick` does nothing; the subsystem exists so that what a
//! scenario declared is *simulation state*: hashed every tick, identical across machines, and
//! readable by presentation through the snapshot path.
//!
//! # Units at the boundary
//!
//! The scenario format stores positions as `f32` and rotations as degrees. Simulation state is
//! `f64` with angles as integer turns ([ADR 0007](../../../docs/adr/0007-simulation-arithmetic.md),
//! decision 5 and its consequences), so both convert exactly once, here: widening `f32` to `f64` is
//! exact for every value, and degrees become a **binary fraction of a revolution** — `u32`, where
//! one full turn is 2^32 — through correctly-rounded operations only, so every machine computes the
//! same integer.

use std::collections::BTreeMap;

use cic_assets::scenario::Scenario;
use cic_assets::templates::TemplateSet;

use crate::command::PlayerId;
use crate::hash::StateHasher;
use crate::id::ObjectId;
use crate::kernel::Kernel;
use crate::subsystem::{Subsystem, TickContext};

/// The name the [`Forces`] subsystem is registered and hashed under.
pub const FORCES: &str = "forces";

/// One activated player: a lockstep seat bound to what the scenario declared for it.
#[derive(Debug, Clone, PartialEq)]
pub struct Player {
    /// The seat, assigned by authored order: the first declared player is seat zero.
    pub slot: PlayerId,
    /// The scenario's stable identifier, which placements and scripts refer to.
    pub id: String,
    /// Team number; slots sharing one start allied, zero is unallied.
    pub team: u32,
    /// Faction template identifier.
    pub faction: String,
    /// Where this player's starting units appear, in metres.
    pub start: [f64; 3],
}

/// One constructed object.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    /// The owning seat, or `None` for neutral scenery.
    pub owner: Option<PlayerId>,
    /// Template identifier, resolved against the template set when M6 defines one.
    pub template: String,
    /// Position in metres.
    pub position: [f64; 3],
    /// Heading as a binary fraction of a revolution: one full turn is 2^32.
    pub rotation: u32,
    /// Uniform scale.
    pub scale: f64,
}

/// The activated scenario as simulation state: players and constructed objects.
#[derive(Debug, Clone, Default)]
pub struct Forces {
    players: Vec<Player>,
    objects: BTreeMap<ObjectId, Placed>,
}

impl Forces {
    /// The activated players, in seat order.
    #[must_use]
    pub fn players(&self) -> &[Player] {
        &self.players
    }

    /// The constructed objects, keyed by identifier.
    #[must_use]
    pub fn objects(&self) -> &BTreeMap<ObjectId, Placed> {
        &self.objects
    }
}

impl Subsystem for Forces {
    fn name(&self) -> &'static str {
        FORCES
    }

    fn tick(&mut self, _context: &mut TickContext<'_>) {
        // Deliberately nothing: what these objects do is gameplay, and gameplay is M6.
    }

    fn write_state(&self, hasher: &mut StateHasher) {
        hasher.write_u64(self.players.len() as u64);
        for player in &self.players {
            hasher.write_bytes(&[player.slot.0]);
            hasher.write_str(&player.id);
            hasher.write_u64(u64::from(player.team));
            hasher.write_str(&player.faction);
            for coordinate in player.start {
                hasher.write_f64(coordinate);
            }
        }
        hasher.write_u64(self.objects.len() as u64);
        for (id, placed) in &self.objects {
            hasher.write_u64(id.0);
            match placed.owner {
                Some(owner) => hasher.write_bytes(&[1, owner.0]),
                None => hasher.write_bytes(&[0, 0]),
            }
            hasher.write_str(&placed.template);
            for coordinate in placed.position {
                hasher.write_f64(coordinate);
            }
            hasher.write_u64(u64::from(placed.rotation));
            hasher.write_f64(placed.scale);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Why a scenario could not be activated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivationError {
    /// More players than lockstep seats.
    TooManyPlayers {
        /// How many the scenario declares.
        declared: usize,
    },
    /// A placement's owner names no declared player.
    ///
    /// [`Scenario::validate`] refuses this too; it is checked again here because activation is the
    /// last line before the reference becomes kernel state, and a caller constructing a scenario in
    /// code bypasses the JSON path validation runs on.
    UnknownOwner {
        /// The placement's index in authored order.
        placement: usize,
        /// The owner it names.
        owner: String,
    },
    /// A placement names a template the set does not define.
    UnknownTemplate {
        /// The placement's index in authored order.
        placement: usize,
        /// The template it names.
        template: String,
    },
    /// A placement names a template whose kind cannot be placed — a faction on the ground.
    NotPlaceable {
        /// The placement's index in authored order.
        placement: usize,
        /// The template it names.
        template: String,
    },
    /// A player names a faction the set does not define.
    UnknownFaction {
        /// The player's scenario identifier.
        player: String,
        /// The faction it names.
        faction: String,
    },
    /// A player's faction resolves to a template that is not a faction.
    NotAFaction {
        /// The player's scenario identifier.
        player: String,
        /// The template it names.
        faction: String,
    },
    /// The kernel already holds an activated scenario.
    AlreadyActivated,
}

impl std::fmt::Display for ActivationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyPlayers { declared } => {
                write!(
                    formatter,
                    "{declared} players, but there are only 256 seats"
                )
            }
            Self::UnknownOwner { placement, owner } => write!(
                formatter,
                "placement {placement} is owned by `{owner}`, which no player declares"
            ),
            Self::UnknownTemplate {
                placement,
                template,
            } => write!(
                formatter,
                "placement {placement} names `{template}`, which the template set does not define"
            ),
            Self::NotPlaceable {
                placement,
                template,
            } => write!(
                formatter,
                "placement {placement} names `{template}`, whose kind cannot be placed"
            ),
            Self::UnknownFaction { player, faction } => write!(
                formatter,
                "player `{player}` names `{faction}`, which the template set does not define"
            ),
            Self::NotAFaction { player, faction } => write!(
                formatter,
                "player `{player}` names `{faction}`, which is not a faction"
            ),
            Self::AlreadyActivated => {
                write!(formatter, "the kernel already holds an activated scenario")
            }
        }
    }
}

impl std::error::Error for ActivationError {}

/// Degrees to a binary fraction of a revolution, exactly once, at the boundary.
///
/// Every step is exact or correctly rounded — the `f32` widens exactly, the reduction is the exact
/// turn-domain subtraction ADR 0007 decision 5 is about, and the scale by 2^32 rounds once — so
/// every machine derives the identical integer from the identical authored value.
fn binary_turns(degrees: f32) -> u32 {
    let turns = f64::from(degrees) / 360.0;
    let reduced = turns - turns.floor();
    let scaled = (reduced * 4_294_967_296.0).round();
    // A fraction just under one can round up to exactly 2^32, which is a whole turn: zero.
    if scaled >= 4_294_967_296.0 {
        0
    } else {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the value was reduced to [0, 2^32) and rounded, so it fits"
        )]
        {
            scaled as u32
        }
    }
}

/// Activates a scenario: players take their seats and every placement becomes an object.
///
/// Seats are assigned in authored order, and object identifiers are allocated in authored order —
/// so the first declared placement is `ObjectId(1)` on every machine, which is what makes a
/// scenario's construction as replayable as everything after it.
///
/// Every reference is resolved against the template set here, because this is the last line before
/// a name becomes kernel state: a placement must name a placeable template, and a player's faction
/// must name a faction. The scenario and the set may come from different mounts, so neither format
/// can check the other on its own — the same reason the package checks positions against the
/// terrain.
///
/// # Errors
///
/// Returns [`ActivationError`] when the scenario declares more players than seats, when a
/// placement's owner names no declared player, when a placement or faction fails to resolve
/// against the template set, or when the kernel already holds an activated scenario.
pub fn activate(
    kernel: &mut Kernel,
    scenario: &Scenario,
    templates: &TemplateSet,
) -> Result<(), ActivationError> {
    if kernel.subsystem(FORCES).is_some() {
        return Err(ActivationError::AlreadyActivated);
    }
    if scenario.players.len() > 256 {
        return Err(ActivationError::TooManyPlayers {
            declared: scenario.players.len(),
        });
    }
    for slot in &scenario.players {
        match templates.get(&slot.faction) {
            None => {
                return Err(ActivationError::UnknownFaction {
                    player: slot.id.clone(),
                    faction: slot.faction.clone(),
                });
            }
            Some(template) if template.kind != cic_assets::templates::TemplateKind::Faction => {
                return Err(ActivationError::NotAFaction {
                    player: slot.id.clone(),
                    faction: slot.faction.clone(),
                });
            }
            Some(_) => {}
        }
    }
    for (index, placement) in scenario.objects.iter().enumerate() {
        match templates.get(&placement.template) {
            None => {
                return Err(ActivationError::UnknownTemplate {
                    placement: index,
                    template: placement.template.clone(),
                });
            }
            Some(template) if !template.kind.placeable() => {
                return Err(ActivationError::NotPlaceable {
                    placement: index,
                    template: placement.template.clone(),
                });
            }
            Some(_) => {}
        }
    }

    let players: Vec<Player> = scenario
        .players
        .iter()
        .enumerate()
        .map(|(seat, slot)| Player {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "the seat count was just checked against the 256-seat bound"
            )]
            slot: PlayerId(seat as u8),
            id: slot.id.clone(),
            team: slot.team,
            faction: slot.faction.clone(),
            start: [
                f64::from(slot.start.x),
                f64::from(slot.start.y),
                f64::from(slot.start.z),
            ],
        })
        .collect();

    let mut objects = BTreeMap::new();
    for (index, placement) in scenario.objects.iter().enumerate() {
        let owner = match &placement.owner {
            None => None,
            Some(name) => Some(
                players
                    .iter()
                    .find(|player| &player.id == name)
                    .map(|player| player.slot)
                    .ok_or_else(|| ActivationError::UnknownOwner {
                        placement: index,
                        owner: name.clone(),
                    })?,
            ),
        };
        let id = kernel.allocate_id();
        objects.insert(
            id,
            Placed {
                owner,
                template: placement.template.clone(),
                position: [
                    f64::from(placement.position.x),
                    f64::from(placement.position.y),
                    f64::from(placement.position.z),
                ],
                rotation: binary_turns(placement.rotation),
                scale: f64::from(placement.scale),
            },
        );
    }

    kernel.add_subsystem(Box::new(Forces { players, objects }));
    Ok(())
}

#[cfg(test)]
mod tests {
    // The conversions under test are exact — widening `f32` to `f64` cannot round — so equality is
    // precisely the property being asserted.
    #![allow(clippy::float_cmp)]

    use cic_assets::scenario::{
        ObjectPlacement, PlayerSlot, Position, Scenario, TerrainReference, Waypoint,
    };
    use cic_assets::templates::{Template, TemplateKind, TemplateSet};

    use super::{ActivationError, FORCES, Forces, activate, binary_turns};
    use crate::command::PlayerId;
    use crate::id::ObjectId;
    use crate::kernel::{Kernel, KernelConfig, first_divergence};

    fn player(id: &str, team: u32) -> PlayerSlot {
        PlayerSlot {
            id: id.to_owned(),
            name: id.to_owned(),
            faction: format!("faction/{id}"),
            start: Position {
                x: 100.0,
                y: 900.0,
                z: 0.0,
            },
            team,
        }
    }

    fn placement(template: &str, owner: Option<&str>) -> ObjectPlacement {
        ObjectPlacement {
            template: template.to_owned(),
            position: Position {
                x: 500.0,
                y: 500.0,
                z: 0.0,
            },
            rotation: 90.0,
            scale: 1.25,
            owner: owner.map(str::to_owned),
        }
    }

    fn scenario() -> Scenario {
        Scenario {
            format_version: 1,
            name: "Alpine Assault".to_owned(),
            description: String::new(),
            terrain: TerrainReference {
                path: "terrain/alpine.cict".to_owned(),
            },
            players: vec![player("north", 1), player("south", 2)],
            objects: vec![
                placement("prop/pine", None),
                placement("structure/depot", Some("north")),
                placement("structure/depot", Some("south")),
            ],
            waypoints: vec![Waypoint {
                name: "centre".to_owned(),
                position: Position {
                    x: 500.0,
                    y: 500.0,
                    z: 0.0,
                },
            }],
            scripts: Vec::new(),
        }
    }

    fn templates() -> TemplateSet {
        let entry = |id: &str, kind, model: Option<&str>| Template {
            id: id.to_owned(),
            kind,
            model: model.map(str::to_owned),
            name: None,
            speed: None,
            footprint: None,
            passage: None,
        };
        TemplateSet {
            format_version: 1,
            templates: vec![
                entry("prop/pine", TemplateKind::Prop, Some("models/pine.glb")),
                entry(
                    "structure/depot",
                    TemplateKind::Structure,
                    Some("models/depot.glb"),
                ),
                entry("faction/north", TemplateKind::Faction, None),
                entry("faction/south", TemplateKind::Faction, None),
            ],
        }
    }

    fn kernel() -> Kernel {
        Kernel::new(KernelConfig {
            seed: 11,
            ticks_per_second: 30,
        })
    }

    #[test]
    fn declared_starts_produce_the_objects_they_claim() {
        // The exit condition's activation half, literally: two declared players seated in authored
        // order, three declared placements constructed with authored-order identifiers, owners
        // resolved to seats, and the pose carried into simulation units.
        let mut kernel = kernel();
        activate(&mut kernel, &scenario(), &templates()).expect("a valid scenario activates");

        let forces = kernel
            .subsystem(FORCES)
            .and_then(|subsystem| subsystem.as_any().downcast_ref::<Forces>())
            .expect("activation registered the forces");

        assert_eq!(forces.players().len(), 2);
        assert_eq!(forces.players()[0].slot, PlayerId(0));
        assert_eq!(forces.players()[0].id, "north");
        assert_eq!(forces.players()[0].team, 1);
        assert_eq!(forces.players()[0].start, [100.0, 900.0, 0.0]);
        assert_eq!(forces.players()[1].slot, PlayerId(1));

        assert_eq!(forces.objects().len(), 3);
        let pine = &forces.objects()[&ObjectId(1)];
        assert_eq!(pine.owner, None);
        assert_eq!(pine.template, "prop/pine");
        let north_depot = &forces.objects()[&ObjectId(2)];
        assert_eq!(north_depot.owner, Some(PlayerId(0)));
        let south_depot = &forces.objects()[&ObjectId(3)];
        assert_eq!(south_depot.owner, Some(PlayerId(1)));
        assert_eq!(south_depot.position, [500.0, 500.0, 0.0]);
        assert_eq!(south_depot.scale, 1.25);
    }

    #[test]
    fn two_machines_activate_identically_and_stay_identical() {
        // Activation is part of the replayable run: the same scenario into two kernels produces the
        // same hashes on every tick that follows.
        let mut ours = kernel();
        let mut theirs = kernel();
        activate(&mut ours, &scenario(), &templates()).expect("activates");
        activate(&mut theirs, &scenario(), &templates()).expect("activates");

        let our_hashes: Vec<_> = (0..10).map(|_| ours.advance(&[]).unwrap()).collect();
        let their_hashes: Vec<_> = (0..10).map(|_| theirs.advance(&[]).unwrap()).collect();
        assert_eq!(first_divergence(&our_hashes, &their_hashes), None);
        assert_eq!(our_hashes, their_hashes);
    }

    #[test]
    fn a_different_scenario_is_a_different_run() {
        // The complement: activation state is *in* the hash, not beside it. One moved placement
        // must diverge on the first tick, attributed to the forces.
        let mut ours = kernel();
        activate(&mut ours, &scenario(), &templates()).expect("activates");

        let mut moved = scenario();
        moved.objects[0].position.x = 501.0;
        let mut theirs = kernel();
        activate(&mut theirs, &moved, &templates()).expect("activates");

        let our_hashes = vec![ours.advance(&[]).unwrap()];
        let their_hashes = vec![theirs.advance(&[]).unwrap()];
        let divergence =
            first_divergence(&our_hashes, &their_hashes).expect("the moved pine must show");
        assert_eq!(divergence.tick, 0);
        assert_eq!(divergence.entry, Some(FORCES));
    }

    #[test]
    fn an_unknown_owner_is_refused_with_its_placement_named() {
        let mut broken = scenario();
        broken.objects[1].owner = Some("east".to_owned());
        let result = activate(&mut kernel(), &broken, &templates());
        assert_eq!(
            result,
            Err(ActivationError::UnknownOwner {
                placement: 1,
                owner: "east".to_owned()
            })
        );
    }

    #[test]
    fn an_unresolved_template_is_refused_with_its_placement_named() {
        let mut unresolved = scenario();
        unresolved.objects[0].template = "prop/oak".to_owned();
        assert_eq!(
            activate(&mut kernel(), &unresolved, &templates()),
            Err(ActivationError::UnknownTemplate {
                placement: 0,
                template: "prop/oak".to_owned()
            })
        );
    }

    #[test]
    fn placing_a_faction_on_the_ground_is_refused() {
        let mut confused = scenario();
        confused.objects[2].template = "faction/north".to_owned();
        assert_eq!(
            activate(&mut kernel(), &confused, &templates()),
            Err(ActivationError::NotPlaceable {
                placement: 2,
                template: "faction/north".to_owned()
            })
        );
    }

    #[test]
    fn an_unresolved_faction_is_refused_with_its_player_named() {
        let mut unresolved = scenario();
        unresolved.players[1].faction = "faction/east".to_owned();
        assert_eq!(
            activate(&mut kernel(), &unresolved, &templates()),
            Err(ActivationError::UnknownFaction {
                player: "south".to_owned(),
                faction: "faction/east".to_owned()
            })
        );
    }

    #[test]
    fn a_player_whose_faction_is_a_prop_is_refused() {
        let mut confused = scenario();
        confused.players[0].faction = "prop/pine".to_owned();
        assert_eq!(
            activate(&mut kernel(), &confused, &templates()),
            Err(ActivationError::NotAFaction {
                player: "north".to_owned(),
                faction: "prop/pine".to_owned()
            })
        );
    }

    #[test]
    fn activating_twice_is_refused() {
        let mut kernel = kernel();
        activate(&mut kernel, &scenario(), &templates()).expect("the first activation succeeds");
        assert_eq!(
            activate(&mut kernel, &scenario(), &templates()),
            Err(ActivationError::AlreadyActivated)
        );
    }

    #[test]
    fn too_many_players_is_refused() {
        let mut crowded = scenario();
        crowded.players = (0..257).map(|n| player(&format!("p{n}"), 0)).collect();
        assert_eq!(
            activate(&mut kernel(), &crowded, &templates()),
            Err(ActivationError::TooManyPlayers { declared: 257 })
        );
    }

    #[test]
    fn degrees_become_exact_binary_turns() {
        // 90 degrees is a quarter turn is exactly 2^30 — every step of the conversion is exact for
        // these values, so the assertion is equality, not tolerance.
        assert_eq!(binary_turns(90.0), 1 << 30);
        assert_eq!(binary_turns(0.0), 0);
        assert_eq!(binary_turns(360.0), 0);
        assert_eq!(binary_turns(-90.0), 3 << 30);
        assert_eq!(binary_turns(450.0), 1 << 30);
    }
}
