//! The authored, human-editable half of a map.
//!
//! # Why JSON and not BSON
//!
//! BSON would buy a smaller file and a slightly faster parse. Both are irrelevant here: the bulk
//! numeric data lives in the terrain container, so a scenario is kilobytes, and the whole package is
//! DEFLATE-compressed anyway — which erases most of BSON's size advantage over JSON's repeated keys.
//!
//! What JSON buys instead is decisive during development: a scenario is **diffable**. A code review
//! can show that a placement moved, `git blame` can attribute a balance change, a merge conflict in
//! two designers' edits is resolvable by hand, and a map can be fixed in a text editor when the tool
//! that wrote it has a bug. None of that survives a binary encoding.
//!
//! Unknown fields are rejected rather than ignored, so a typo in a hand-edited scenario is a loud
//! error at load rather than a silently-defaulted value that shows up as a gameplay bug.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use serde::{Deserialize, Serialize};

/// The scenario schema version this build writes and reads.
pub const FORMAT_VERSION: u32 = 1;

/// A world position in metres, Z up.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Position {
    /// Eastward offset.
    pub x: f32,
    /// Northward offset.
    pub y: f32,
    /// Elevation. When absent from the file it defaults to zero, meaning "sit on the terrain".
    #[serde(default)]
    pub z: f32,
}

/// One playable slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerSlot {
    /// Stable identifier referenced by placements.
    pub id: String,
    /// Display name for the lobby.
    pub name: String,
    /// Faction template identifier, resolved against the template set.
    pub faction: String,
    /// Where this player's starting units appear.
    pub start: Position,
    /// Team number. Slots sharing a team start allied; `0` means unallied.
    #[serde(default)]
    pub team: u32,
}

/// One placed world object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectPlacement {
    /// Template identifier, resolved against the template set.
    pub template: String,
    /// Where the object sits.
    pub position: Position,
    /// Rotation about Z in degrees.
    #[serde(default)]
    pub rotation: f32,
    /// Uniform scale. Defaults to `1.0`.
    #[serde(default = "one")]
    pub scale: f32,
    /// Owning player slot id, or absent for neutral scenery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

const fn one() -> f32 {
    1.0
}

/// A named position designers and scripts refer to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Waypoint {
    /// Unique name.
    pub name: String,
    /// Where it is.
    pub position: Position,
}

/// Where the terrain container lives inside the package, and how it maps to the world.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerrainReference {
    /// Package-relative path of the terrain container.
    pub path: String,
}

/// The authored description of one map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// Schema version, so a future change can be detected rather than mis-parsed.
    pub format_version: u32,
    /// Map name shown in the lobby.
    pub name: String,
    /// Longer description.
    #[serde(default)]
    pub description: String,
    /// The terrain this scenario sits on.
    pub terrain: TerrainReference,
    /// Playable slots, in lobby order.
    pub players: Vec<PlayerSlot>,
    /// Placed world objects.
    #[serde(default)]
    pub objects: Vec<ObjectPlacement>,
    /// Named positions.
    #[serde(default)]
    pub waypoints: Vec<Waypoint>,
    /// Package-relative paths of the scripts this scenario runs, in dispatch order.
    ///
    /// Ordered because the order is the contract: when several scripts handle the same event they
    /// run in this sequence, so it is something a designer changes deliberately rather than
    /// something derived from a directory listing. A script not named here does not run, however it
    /// got into the package — see [ADR 7002](../../../docs/adr/7002-script-events.md).
    #[serde(default)]
    pub scripts: Vec<String>,
}

impl Scenario {
    /// Parses a scenario from JSON.
    ///
    /// # Errors
    ///
    /// Returns [`ScenarioError::Json`] for malformed JSON, an unknown field, or a missing required
    /// field, then whatever [`Self::validate`] reports.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ScenarioError> {
        let scenario: Self = serde_json::from_slice(bytes).map_err(ScenarioError::Json)?;
        scenario.validate()?;
        Ok(scenario)
    }

    /// Serializes the scenario as pretty-printed JSON.
    ///
    /// Pretty rather than compact on purpose: the package compresses it, and a one-placement-per-few
    /// -lines layout is what makes a diff readable.
    ///
    /// # Errors
    ///
    /// Returns [`ScenarioError::Json`] if serialization fails.
    pub fn to_json(&self) -> Result<Vec<u8>, ScenarioError> {
        let mut bytes = serde_json::to_vec_pretty(self).map_err(ScenarioError::Json)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Checks the invariants JSON's shape cannot express.
    ///
    /// # Errors
    ///
    /// Returns a structured [`ScenarioError`] for an unsupported version, an empty name, no players,
    /// a duplicate player or waypoint identifier, a placement owned by an unknown player, or a
    /// non-finite coordinate.
    pub fn validate(&self) -> Result<(), ScenarioError> {
        if self.format_version != FORMAT_VERSION {
            return Err(ScenarioError::UnsupportedVersion(self.format_version));
        }
        if self.name.trim().is_empty() {
            return Err(ScenarioError::EmptyName);
        }
        if self.terrain.path.trim().is_empty() {
            return Err(ScenarioError::EmptyTerrainPath);
        }
        if self.players.is_empty() {
            return Err(ScenarioError::NoPlayers);
        }

        let mut ids = BTreeSet::new();
        for player in &self.players {
            if player.id.trim().is_empty() {
                return Err(ScenarioError::EmptyPlayerId);
            }
            if !ids.insert(player.id.as_str()) {
                return Err(ScenarioError::DuplicatePlayer(player.id.clone()));
            }
            finite(player.start, || {
                ScenarioError::NonFinitePosition(format!("player {}", player.id))
            })?;
        }

        let mut waypoints = BTreeSet::new();
        for waypoint in &self.waypoints {
            if !waypoints.insert(waypoint.name.as_str()) {
                return Err(ScenarioError::DuplicateWaypoint(waypoint.name.clone()));
            }
            finite(waypoint.position, || {
                ScenarioError::NonFinitePosition(format!("waypoint {}", waypoint.name))
            })?;
        }

        let mut scripts = BTreeSet::new();
        for (index, path) in self.scripts.iter().enumerate() {
            if path.trim().is_empty() {
                return Err(ScenarioError::EmptyScriptPath(index));
            }
            // A repeat would compile and dispatch the same handlers twice, which is never what an
            // author meant and is invisible in the run -- the events simply fire twice.
            if !scripts.insert(path.as_str()) {
                return Err(ScenarioError::DuplicateScript(path.clone()));
            }
        }

        for (index, object) in self.objects.iter().enumerate() {
            if object.template.trim().is_empty() {
                return Err(ScenarioError::EmptyTemplate(index));
            }
            if let Some(owner) = &object.owner
                && !ids.contains(owner.as_str())
            {
                return Err(ScenarioError::UnknownOwner {
                    index,
                    owner: owner.clone(),
                });
            }
            if !object.scale.is_finite() || object.scale <= 0.0 {
                return Err(ScenarioError::InvalidScale {
                    index,
                    scale: object.scale,
                });
            }
            if !object.rotation.is_finite() {
                return Err(ScenarioError::NonFinitePosition(format!(
                    "object {index} rotation"
                )));
            }
            finite(object.position, || {
                ScenarioError::NonFinitePosition(format!("object {index}"))
            })?;
        }
        Ok(())
    }
}

fn finite(position: Position, error: impl FnOnce() -> ScenarioError) -> Result<(), ScenarioError> {
    if position.x.is_finite() && position.y.is_finite() && position.z.is_finite() {
        Ok(())
    } else {
        Err(error())
    }
}

/// A structured failure while loading or validating a scenario.
#[derive(Debug)]
pub enum ScenarioError {
    /// JSON was malformed, held an unknown field, or omitted a required one.
    Json(serde_json::Error),
    /// The scenario declared a schema version this build does not implement.
    UnsupportedVersion(u32),
    /// The map name was blank.
    EmptyName,
    /// The terrain path was blank.
    EmptyTerrainPath,
    /// No playable slots were declared.
    NoPlayers,
    /// A player slot had a blank identifier.
    EmptyPlayerId,
    /// Two player slots shared an identifier.
    DuplicatePlayer(String),
    /// Two waypoints shared a name.
    DuplicateWaypoint(String),
    /// A placement named a blank template.
    EmptyTemplate(usize),
    /// A script entry was blank.
    EmptyScriptPath(usize),
    /// The same script was listed twice.
    DuplicateScript(String),
    /// A placement was owned by a player the scenario does not declare.
    UnknownOwner {
        /// Zero-based placement index.
        index: usize,
        /// The unresolved owner identifier.
        owner: String,
    },
    /// A placement's scale was zero, negative, or not finite.
    InvalidScale {
        /// Zero-based placement index.
        index: usize,
        /// The offending scale.
        scale: f32,
    },
    /// A coordinate was not finite.
    NonFinitePosition(String),
}

impl Display for ScenarioError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => Display::fmt(error, formatter),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported scenario format version {version}")
            }
            Self::EmptyName => formatter.write_str("scenario name is empty"),
            Self::EmptyTerrainPath => formatter.write_str("scenario terrain path is empty"),
            Self::NoPlayers => formatter.write_str("scenario declares no players"),
            Self::EmptyPlayerId => formatter.write_str("a player slot has an empty id"),
            Self::DuplicatePlayer(id) => write!(formatter, "duplicate player id {id}"),
            Self::DuplicateWaypoint(name) => write!(formatter, "duplicate waypoint name {name}"),
            Self::EmptyTemplate(index) => {
                write!(formatter, "object {index} names an empty template")
            }
            Self::EmptyScriptPath(index) => {
                write!(formatter, "script {index} has an empty path")
            }
            Self::DuplicateScript(path) => write!(formatter, "duplicate script path {path}"),
            Self::UnknownOwner { index, owner } => {
                write!(
                    formatter,
                    "object {index} is owned by unknown player {owner}"
                )
            }
            Self::InvalidScale { index, scale } => {
                write!(formatter, "object {index} has invalid scale {scale}")
            }
            Self::NonFinitePosition(what) => {
                write!(formatter, "{what} has a non-finite coordinate")
            }
        }
    }
}

impl Error for ScenarioError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    // Every float compared here is an exactly-representable constant the fixtures set directly
    // (0.0, 1.0, 10.0, 0.9, ...), so exact comparison is the correct assertion -- an epsilon would
    // weaken these tests rather than make them robust.
    #![allow(clippy::float_cmp)]
    use super::{
        FORMAT_VERSION, ObjectPlacement, PlayerSlot, Position, Scenario, ScenarioError,
        TerrainReference, Waypoint,
    };

    fn sample() -> Scenario {
        Scenario {
            format_version: FORMAT_VERSION,
            name: "Alpine Assault".to_owned(),
            description: "Two-player mountain pass.".to_owned(),
            terrain: TerrainReference {
                path: "terrain/alpine.cict".to_owned(),
            },
            players: vec![
                PlayerSlot {
                    id: "north".to_owned(),
                    name: "North".to_owned(),
                    faction: "faction/vanguard".to_owned(),
                    start: Position {
                        x: 100.0,
                        y: 900.0,
                        z: 0.0,
                    },
                    team: 1,
                },
                PlayerSlot {
                    id: "south".to_owned(),
                    name: "South".to_owned(),
                    faction: "faction/coalition".to_owned(),
                    start: Position {
                        x: 900.0,
                        y: 100.0,
                        z: 0.0,
                    },
                    team: 2,
                },
            ],
            objects: vec![ObjectPlacement {
                template: "prop/pine".to_owned(),
                position: Position {
                    x: 500.0,
                    y: 500.0,
                    z: 0.0,
                },
                rotation: 45.0,
                scale: 1.25,
                owner: None,
            }],
            waypoints: vec![Waypoint {
                name: "centre".to_owned(),
                position: Position {
                    x: 500.0,
                    y: 500.0,
                    z: 0.0,
                },
            }],
            scripts: vec!["scripts/mission.cics".to_owned()],
        }
    }

    #[test]
    fn round_trips_through_json() {
        let scenario = sample();
        let json = scenario.to_json().expect("serialize");
        let parsed = Scenario::from_json(&json).expect("parse");
        assert_eq!(parsed, scenario);
    }

    #[test]
    fn serializes_as_readable_diffable_text() {
        let json = String::from_utf8(sample().to_json().expect("serialize")).expect("utf8");
        assert!(json.contains("\n  \"name\": \"Alpine Assault\""), "{json}");
        assert!(json.ends_with('\n'), "a trailing newline keeps diffs clean");
        // A neutral placement omits `owner` rather than writing `null`, so the file stays terse.
        assert!(!json.contains("\"owner\""), "{json}");
    }

    #[test]
    fn applies_documented_defaults_for_omitted_fields() {
        let json = br#"{
            "format_version": 1,
            "name": "Minimal",
            "terrain": { "path": "t.cict" },
            "players": [
                { "id": "a", "name": "A", "faction": "f", "start": { "x": 0.0, "y": 0.0 } }
            ],
            "objects": [ { "template": "prop/rock", "position": { "x": 1.0, "y": 2.0 } } ]
        }"#;
        let scenario = Scenario::from_json(json).expect("parse");
        assert_eq!(scenario.description, "");
        assert_eq!(scenario.players[0].team, 0);
        assert_eq!(scenario.players[0].start.z, 0.0);
        assert_eq!(scenario.objects[0].scale, 1.0, "scale defaults to 1");
        assert_eq!(scenario.objects[0].rotation, 0.0);
        assert!(scenario.waypoints.is_empty());
        assert!(
            scenario.scripts.is_empty(),
            "a scenario without scripts is the ordinary case, not an error"
        );
    }

    #[test]
    fn keeps_the_authored_script_order() {
        // The order is the dispatch order, so it must survive a round trip unsorted.
        let json = br#"{
            "format_version": 1,
            "name": "Scripted",
            "terrain": { "path": "t.cict" },
            "players": [
                { "id": "a", "name": "A", "faction": "f", "start": { "x": 0.0, "y": 0.0 } }
            ],
            "scripts": ["scripts/zulu.cics", "scripts/alpha.cics"]
        }"#;
        let scenario = Scenario::from_json(json).expect("parse");
        assert_eq!(
            scenario.scripts,
            ["scripts/zulu.cics", "scripts/alpha.cics"]
        );
        let reparsed =
            Scenario::from_json(&scenario.to_json().expect("serialize")).expect("reparse");
        assert_eq!(reparsed.scripts, scenario.scripts);
    }

    #[test]
    fn rejects_a_blank_or_repeated_script_path() {
        let mut scenario = sample();
        scenario.scripts = vec!["  ".to_owned()];
        assert!(matches!(
            scenario.validate(),
            Err(ScenarioError::EmptyScriptPath(0))
        ));

        // Listing one twice would compile and dispatch its handlers twice, invisibly.
        scenario.scripts = vec![
            "scripts/mission.cics".to_owned(),
            "scripts/mission.cics".to_owned(),
        ];
        let error = scenario.validate().expect_err("must refuse");
        assert!(matches!(error, ScenarioError::DuplicateScript(_)));
        assert_eq!(
            error.to_string(),
            "duplicate script path scripts/mission.cics"
        );
    }

    #[test]
    fn rejects_an_unknown_field_rather_than_ignoring_it() {
        // A typo in a hand-edited scenario must be loud, not silently defaulted.
        let json = br#"{
            "format_version": 1,
            "name": "Typo",
            "terrain": { "path": "t.cict" },
            "playerz": [],
            "players": [
                { "id": "a", "name": "A", "faction": "f", "start": { "x": 0.0, "y": 0.0 } }
            ]
        }"#;
        let error = Scenario::from_json(json).expect_err("must refuse");
        assert!(matches!(error, ScenarioError::Json(_)), "got {error:?}");
    }

    #[test]
    fn rejects_a_placement_owned_by_an_undeclared_player() {
        let mut scenario = sample();
        scenario.objects[0].owner = Some("nobody".to_owned());
        let error = scenario.validate().expect_err("must refuse");
        assert!(
            matches!(error, ScenarioError::UnknownOwner { ref owner, .. } if owner == "nobody"),
            "got {error:?}"
        );
    }

    #[test]
    fn accepts_a_placement_owned_by_a_declared_player() {
        let mut scenario = sample();
        scenario.objects[0].owner = Some("north".to_owned());
        scenario.validate().expect("owner resolves");
    }

    #[test]
    fn rejects_duplicate_player_and_waypoint_identifiers() {
        let mut scenario = sample();
        scenario.players[1].id = "north".to_owned();
        assert!(matches!(
            scenario.validate(),
            Err(ScenarioError::DuplicatePlayer(_))
        ));

        let mut scenario = sample();
        scenario.waypoints.push(Waypoint {
            name: "centre".to_owned(),
            position: Position {
                x: 1.0,
                y: 1.0,
                z: 0.0,
            },
        });
        assert!(matches!(
            scenario.validate(),
            Err(ScenarioError::DuplicateWaypoint(_))
        ));
    }

    #[test]
    fn rejects_non_finite_coordinates_and_bad_scales() {
        for bad in [f32::NAN, f32::INFINITY] {
            let mut scenario = sample();
            scenario.objects[0].position.x = bad;
            assert!(
                matches!(
                    scenario.validate(),
                    Err(ScenarioError::NonFinitePosition(_))
                ),
                "{bad} must be refused"
            );
        }
        for bad in [0.0f32, -1.0, f32::NAN] {
            let mut scenario = sample();
            scenario.objects[0].scale = bad;
            assert!(
                matches!(scenario.validate(), Err(ScenarioError::InvalidScale { .. })),
                "{bad} must be refused"
            );
        }
    }

    #[test]
    fn rejects_an_empty_name_no_players_and_a_wrong_version() {
        let mut scenario = sample();
        scenario.name = "   ".to_owned();
        assert!(matches!(scenario.validate(), Err(ScenarioError::EmptyName)));

        let mut scenario = sample();
        scenario.players.clear();
        assert!(matches!(scenario.validate(), Err(ScenarioError::NoPlayers)));

        let mut scenario = sample();
        scenario.format_version = 99;
        assert!(matches!(
            scenario.validate(),
            Err(ScenarioError::UnsupportedVersion(99))
        ));
    }
}
