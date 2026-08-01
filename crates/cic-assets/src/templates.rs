//! The template set: what a `template:` identifier resolves to.
//!
//! A scenario places `structure/depot` at a position and gives a player `faction/vanguard`; this
//! format is where those names stop being strings. [M6](../../../docs/milestones/m6-gameplay.md)
//! deferred the format from M2 on purpose — *written once its consumers are known* — and the first
//! consumers now exist: scenario activation resolves every placement and faction against a set, and
//! a drawing host looks a placed object's model up by its template.
//!
//! # Deliberately minimal, and how it grows
//!
//! A template today is an identifier, a kind, a model for the kinds that are drawn, a display name
//! key, a speed, and the two rectangles it stamps on the passability grid. Health, cost and weapons
//! arrive with the M6 mechanics that read them — a field nothing consumes is a field nothing tests,
//! which is the same reasoning that deferred the whole format. Adding an optional field later does
//! not break existing files; changing what an existing field means takes a version bump.
//!
//! `speed`, `footprint` and `passage` are the rule working: each landed with its consumer, movement
//! for the first and [ADR 3001](../../../docs/adr/3001-pathfinding.md) decision 4's grid stamps for
//! the other two.
//!
//! # One document, overridden wholesale
//!
//! A template set is one JSON document at a well-known path (`templates.json`), so the resource
//! layer's ordered mounts apply to it exactly as to any other file: a map package or a mod that
//! provides its own replaces the one beneath it entirely. Per-template merging across mounts is a
//! modding question for later, and it should be decided deliberately rather than fallen into —
//! wholesale replacement is at least never surprising.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// The one schema version this build reads.
const FORMAT_VERSION: u32 = 1;

/// What a template describes, which decides what its fields must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateKind {
    /// A mobile object.
    Unit,
    /// A placed building.
    Structure,
    /// Scenery: placeable, drawable, and owning nothing.
    Prop,
    /// A playable side. Not placeable and not drawn; referenced by a player slot.
    Faction,
}

impl TemplateKind {
    /// Whether a scenario placement may name a template of this kind.
    #[must_use]
    pub fn placeable(self) -> bool {
        !matches!(self, Self::Faction)
    }

    /// Whether an object of this kind may stamp the passability grid it stands on.
    ///
    /// A structure denies the ground beneath it and a bridge grants passage over water; a **unit**
    /// does neither. A footprint that moves would have to be lifted and re-laid every tick, and
    /// [ADR 3001](../../../docs/adr/3001-pathfinding.md) reserves a mover's own occupancy for
    /// decision 10's `radius` and the steering that reads it. A faction has no ground at all.
    #[must_use]
    pub fn stamps(self) -> bool {
        matches!(self, Self::Structure | Self::Prop)
    }
}

/// A rectangle of whole cells, measured along the template's own axes.
///
/// Whole cells and a rectangle are both [ADR
/// 3001](../../../docs/adr/3001-pathfinding.md) decision 4: an object's *visual* rotation stays
/// free, and only the stamp quantizes — to the cell grid, and to quarter turns. Rasterizing a
/// rectangle at 37° deterministically is solvable and buys nothing a genre whose buildings have
/// snapped to grids since it existed would notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Footprint {
    /// Extent along the template's own X and Y axes, in whole cells. Neither may be zero.
    pub cells: [u32; 2],
}

/// The cells an object makes traversable, and what crossing them costs.
///
/// A bridge grants passage over water it spans, a tunnel through ground the slope test refused, a
/// ramp at a grade. Passage **overrides the derivation**: elevation stays presentation's problem,
/// so a unit on a bridge is a unit whose cells happen to be passable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Passage {
    /// Extent along the template's own X and Y axes, in whole cells. Neither may be zero.
    pub cells: [u32; 2],
    /// The cost class the granted cells take, on the ladder `cic_sim::ground` names: `1` metalled,
    /// `2` graded, `3` plain, `4` mud, `5` rubble. Zero is the impassable class and is refused —
    /// something that denies ground declares a `footprint`, which is what the word means.
    pub class: u8,
}

/// One template.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    /// The identifier scenarios reference, unique within the set.
    pub id: String,
    /// What this template describes.
    pub kind: TemplateKind,
    /// Package-relative path of the model that draws it. Required for a placeable kind, refused for
    /// a faction — a faction has no pose to draw a model at, and a file claiming one is confused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// String-table key for the display name, when one exists to show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Movement speed in world units per second. Required for a `unit` — a unit that cannot move is
    /// a structure wearing the wrong kind — and refused for everything else, which has no movement
    /// for it to mean anything to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,
    /// How much room the unit takes up, as a radius in world units. Required for a `unit` and
    /// refused for everything else, on the same rule as [`Self::speed`].
    ///
    /// This is what keeps units from standing in each other —
    /// [ADR 3001](../../../docs/adr/3001-pathfinding.md) decision 10. A **standing** object's
    /// occupancy is its [`Self::footprint`] instead: a structure denies whole cells and never moves,
    /// so it belongs to the grid, while a mover is a circle that other movers push away from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    /// The cells this object occupies. While it stands, they are impassable whatever the terrain
    /// says. Optional, and only a kind that [stamps](TemplateKind::stamps) may declare one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footprint: Option<Footprint>,
    /// The cells this object makes traversable, and at what cost. Optional, same kinds as
    /// [`Self::footprint`].
    ///
    /// A template may declare both: a gatehouse is a bridge with a building on it, and the
    /// precedence — derivation, then passage, then occlusion — resolves the overlap by denying it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passage: Option<Passage>,
}

/// The template set: every template this content defines, keyed by identifier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateSet {
    /// Schema version, so a future change is detected rather than mis-parsed.
    pub format_version: u32,
    /// The templates, in authored order.
    pub templates: Vec<Template>,
}

impl TemplateSet {
    /// Parses a template set from JSON.
    ///
    /// # Errors
    ///
    /// Returns [`TemplateError::Json`] for malformed JSON, an unknown field, or a missing required
    /// field, then whatever [`Self::validate`] reports.
    pub fn from_json(bytes: &[u8]) -> Result<Self, TemplateError> {
        let set: Self = serde_json::from_slice(bytes).map_err(TemplateError::Json)?;
        set.validate()?;
        Ok(set)
    }

    /// Serializes the set as pretty-printed JSON with a trailing newline, for the same diffability
    /// reason the scenario does.
    ///
    /// # Errors
    ///
    /// Returns [`TemplateError::Json`] if serialization fails.
    pub fn to_json(&self) -> Result<Vec<u8>, TemplateError> {
        let mut bytes = serde_json::to_vec_pretty(self).map_err(TemplateError::Json)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Checks the invariants JSON's shape cannot express.
    ///
    /// # Errors
    ///
    /// Returns a structured [`TemplateError`] for an unsupported version, a blank or duplicate
    /// identifier, a placeable template with no model or a blank one, a faction with a model, a
    /// speed on something that cannot move, or a grid stamp that is empty, impassable, or on a kind
    /// that does not stamp.
    pub fn validate(&self) -> Result<(), TemplateError> {
        if self.format_version != FORMAT_VERSION {
            return Err(TemplateError::UnsupportedVersion(self.format_version));
        }
        let mut ids = BTreeSet::new();
        for template in &self.templates {
            if template.id.trim().is_empty() {
                return Err(TemplateError::EmptyId);
            }
            if !ids.insert(template.id.as_str()) {
                return Err(TemplateError::DuplicateId(template.id.clone()));
            }
            match (&template.model, template.kind.placeable()) {
                (None, true) => {
                    return Err(TemplateError::MissingModel(template.id.clone()));
                }
                (Some(model), true) if model.trim().is_empty() => {
                    return Err(TemplateError::MissingModel(template.id.clone()));
                }
                (Some(_), false) => {
                    return Err(TemplateError::FactionWithModel(template.id.clone()));
                }
                _ => {}
            }
            // Speed and radius are the same rule twice, so the rule is written once: whichever way
            // one of them can be wrong, the other can be wrong the same way, and two copies of a
            // three-armed match are two things that can drift apart.
            match mover_only(template.speed, template.kind) {
                Some(MoverFault::Missing) => {
                    return Err(TemplateError::MissingSpeed(template.id.clone()));
                }
                Some(MoverFault::Invalid(speed)) => {
                    return Err(TemplateError::InvalidSpeed {
                        id: template.id.clone(),
                        speed,
                    });
                }
                Some(MoverFault::Misplaced) => {
                    return Err(TemplateError::SpeedOnNonUnit(template.id.clone()));
                }
                None => {}
            }
            match mover_only(template.radius, template.kind) {
                Some(MoverFault::Missing) => {
                    return Err(TemplateError::MissingRadius(template.id.clone()));
                }
                Some(MoverFault::Invalid(radius)) => {
                    return Err(TemplateError::InvalidRadius {
                        id: template.id.clone(),
                        radius,
                    });
                }
                Some(MoverFault::Misplaced) => {
                    return Err(TemplateError::RadiusOnNonUnit(template.id.clone()));
                }
                None => {}
            }

            let extents = [
                template.footprint.map(|shape| shape.cells),
                template.passage.map(|shape| shape.cells),
            ];
            if extents.iter().any(Option::is_some) && !template.kind.stamps() {
                return Err(TemplateError::UnstampableKind(template.id.clone()));
            }
            if extents
                .into_iter()
                .flatten()
                .any(|cells| cells.contains(&0))
            {
                return Err(TemplateError::EmptyStamp(template.id.clone()));
            }
            if template.passage.is_some_and(|passage| passage.class == 0) {
                return Err(TemplateError::ImpassablePassage(template.id.clone()));
            }
        }
        Ok(())
    }

    /// The template with the given identifier.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Template> {
        self.templates.iter().find(|template| template.id == id)
    }
}

/// How a measurement only a `unit` may carry is wrong, if it is.
enum MoverFault {
    /// A unit declared none.
    Missing,
    /// A unit declared one that is zero, negative, or not a number.
    Invalid(f32),
    /// Something that is not a unit declared one.
    Misplaced,
}

/// Checks a field that a `unit` must have a positive finite value for and nothing else may have.
///
/// `speed` and `radius` are both this rule; see the two call sites in [`TemplateSet::validate`] for
/// why it is written here rather than twice.
fn mover_only(value: Option<f32>, kind: TemplateKind) -> Option<MoverFault> {
    match (value, kind) {
        (None, TemplateKind::Unit) => Some(MoverFault::Missing),
        (Some(value), TemplateKind::Unit) if !(value.is_finite() && value > 0.0) => {
            Some(MoverFault::Invalid(value))
        }
        (Some(_), kind) if kind != TemplateKind::Unit => Some(MoverFault::Misplaced),
        _ => None,
    }
}

/// A structured failure while loading or validating a template set.
#[derive(Debug)]
pub enum TemplateError {
    /// JSON was malformed, held an unknown field, or omitted a required one.
    Json(serde_json::Error),
    /// The set declared a schema version this build does not implement.
    UnsupportedVersion(u32),
    /// A template had a blank identifier.
    EmptyId,
    /// Two templates shared an identifier.
    DuplicateId(String),
    /// A placeable template declared no model, or a blank one.
    MissingModel(String),
    /// A faction declared a model, which nothing can draw.
    FactionWithModel(String),
    /// A unit declared no speed, and a unit that cannot move is a structure wearing the wrong kind.
    MissingSpeed(String),
    /// A unit's speed was zero, negative, or not finite.
    InvalidSpeed {
        /// The template.
        id: String,
        /// The offending value.
        speed: f32,
    },
    /// Something that is not a unit declared a speed, which has no movement to mean anything to.
    SpeedOnNonUnit(String),
    /// A unit declared no radius, so nothing knows how much room to keep around it.
    MissingRadius(String),
    /// A unit's radius was zero, negative, or not finite.
    InvalidRadius {
        /// The template.
        id: String,
        /// The offending value.
        radius: f32,
    },
    /// Something that is not a unit declared a radius; a standing object occupies cells instead.
    RadiusOnNonUnit(String),
    /// A kind that cannot stamp the grid declared a `footprint` or a `passage`.
    UnstampableKind(String),
    /// A `footprint` or `passage` declared a rectangle with a zero extent, which covers no ground.
    EmptyStamp(String),
    /// A `passage` declared cost class zero, which is the class nothing may enter.
    ImpassablePassage(String),
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(error) => write!(formatter, "template JSON: {error}"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "template set version {version} is not supported")
            }
            Self::EmptyId => write!(formatter, "a template has a blank identifier"),
            Self::DuplicateId(id) => write!(formatter, "template `{id}` is declared twice"),
            Self::MissingModel(id) => write!(
                formatter,
                "template `{id}` is placeable but declares no model"
            ),
            Self::FactionWithModel(id) => write!(
                formatter,
                "faction `{id}` declares a model, which nothing can draw"
            ),
            Self::MissingSpeed(id) => {
                write!(formatter, "unit `{id}` declares no speed and cannot move")
            }
            Self::InvalidSpeed { id, speed } => write!(
                formatter,
                "unit `{id}` declares speed {speed}, which is not a positive finite number"
            ),
            Self::SpeedOnNonUnit(id) => write!(
                formatter,
                "template `{id}` declares a speed but is not a unit"
            ),
            Self::MissingRadius(id) => write!(
                formatter,
                "unit `{id}` declares no radius, so nothing knows how much room to keep around it"
            ),
            Self::InvalidRadius { id, radius } => write!(
                formatter,
                "unit `{id}` declares radius {radius}, which is not a positive finite number"
            ),
            Self::RadiusOnNonUnit(id) => write!(
                formatter,
                "template `{id}` declares a radius but is not a unit; a standing object occupies \
                 cells with a footprint instead"
            ),
            Self::UnstampableKind(id) => write!(
                formatter,
                "template `{id}` declares a footprint or a passage, which only a structure or a \
                 prop may stamp"
            ),
            Self::EmptyStamp(id) => write!(
                formatter,
                "template `{id}` declares a stamp with a zero extent, which covers no ground"
            ),
            Self::ImpassablePassage(id) => write!(
                formatter,
                "the passage on `{id}` declares cost class 0, which is impassable — a footprint is \
                 what denies ground"
            ),
        }
    }
}

impl std::error::Error for TemplateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Footprint, Passage, Template, TemplateError, TemplateKind, TemplateSet};

    fn template(id: &str, kind: TemplateKind, model: Option<&str>) -> Template {
        Template {
            id: id.to_owned(),
            kind,
            model: model.map(str::to_owned),
            name: None,
            speed: None,
            radius: None,
            footprint: None,
            passage: None,
        }
    }

    /// A legal one-unit set, with whatever the caller wants to break about it.
    fn unit_set(speed: Option<f32>, radius: Option<f32>) -> TemplateSet {
        let mut entry = template("unit/rifleman", TemplateKind::Unit, Some("models/r.glb"));
        entry.speed = speed;
        entry.radius = radius;
        TemplateSet {
            format_version: 1,
            templates: vec![entry],
        }
    }

    fn set() -> TemplateSet {
        TemplateSet {
            format_version: 1,
            templates: vec![
                template("prop/pine", TemplateKind::Prop, Some("models/pine.glb")),
                template(
                    "structure/depot",
                    TemplateKind::Structure,
                    Some("models/depot.glb"),
                ),
                template("faction/vanguard", TemplateKind::Faction, None),
            ],
        }
    }

    #[test]
    fn a_round_trip_preserves_the_set_and_stays_diffable() {
        let bytes = set().to_json().expect("serializes");
        assert_eq!(bytes.last(), Some(&b'\n'));
        let read = TemplateSet::from_json(&bytes).expect("parses");
        assert_eq!(read, set());
        assert_eq!(
            read.get("prop/pine").map(|t| t.kind),
            Some(TemplateKind::Prop)
        );
        assert_eq!(read.get("prop/oak"), None);
    }

    #[test]
    fn an_unknown_field_is_refused_loudly() {
        let json = br#"{ "format_version": 1, "templates": [], "surprise": true }"#;
        assert!(matches!(
            TemplateSet::from_json(json),
            Err(TemplateError::Json(_))
        ));
    }

    #[test]
    fn a_wrong_version_is_refused() {
        let mut wrong = set();
        wrong.format_version = 2;
        assert!(matches!(
            wrong.validate(),
            Err(TemplateError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn a_duplicate_identifier_is_refused() {
        let mut doubled = set();
        doubled
            .templates
            .push(template("prop/pine", TemplateKind::Prop, Some("m.glb")));
        assert!(matches!(
            doubled.validate(),
            Err(TemplateError::DuplicateId(id)) if id == "prop/pine"
        ));
    }

    #[test]
    fn a_placeable_template_needs_a_model_and_a_faction_must_not_have_one() {
        let mut modelless = set();
        modelless.templates[0].model = None;
        assert!(matches!(
            modelless.validate(),
            Err(TemplateError::MissingModel(id)) if id == "prop/pine"
        ));

        let mut blank = set();
        blank.templates[1].model = Some("  ".to_owned());
        assert!(matches!(
            blank.validate(),
            Err(TemplateError::MissingModel(id)) if id == "structure/depot"
        ));

        let mut confused = set();
        confused.templates[2].model = Some("models/flag.glb".to_owned());
        assert!(matches!(
            confused.validate(),
            Err(TemplateError::FactionWithModel(id)) if id == "faction/vanguard"
        ));
    }

    #[test]
    fn a_unit_needs_a_positive_finite_speed_and_nothing_else_may_have_one() {
        let unit = |speed| unit_set(speed, Some(2.0));
        assert!(unit(Some(3.5)).validate().is_ok());
        assert!(matches!(
            unit(None).validate(),
            Err(TemplateError::MissingSpeed(id)) if id == "unit/rifleman"
        ));
        assert!(matches!(
            unit(Some(0.0)).validate(),
            Err(TemplateError::InvalidSpeed { .. })
        ));
        assert!(matches!(
            unit(Some(f32::NAN)).validate(),
            Err(TemplateError::InvalidSpeed { .. })
        ));

        let mut rolling_depot = set();
        rolling_depot.templates[1].speed = Some(2.0);
        assert!(matches!(
            rolling_depot.validate(),
            Err(TemplateError::SpeedOnNonUnit(id)) if id == "structure/depot"
        ));
    }

    #[test]
    fn a_unit_needs_a_positive_finite_radius_and_nothing_else_may_have_one() {
        // The same rule as `speed`, asserted separately on purpose: the two share one check, and a
        // shared check is worth having only if both sides of it are pinned. A radius of zero is the
        // interesting refusal — it parses, it reads as "takes up no room", and what it would
        // actually mean is a unit nothing can ever push, standing inside its neighbours for ever.
        let unit = |radius| unit_set(Some(3.5), radius);
        assert!(unit(Some(2.0)).validate().is_ok());
        assert!(matches!(
            unit(None).validate(),
            Err(TemplateError::MissingRadius(id)) if id == "unit/rifleman"
        ));
        assert!(matches!(
            unit(Some(0.0)).validate(),
            Err(TemplateError::InvalidRadius { .. })
        ));
        assert!(matches!(
            unit(Some(-1.0)).validate(),
            Err(TemplateError::InvalidRadius { .. })
        ));
        assert!(matches!(
            unit(Some(f32::INFINITY)).validate(),
            Err(TemplateError::InvalidRadius { .. })
        ));

        // A structure keeps its room with a footprint, which is whole cells and does not move.
        let mut wide_depot = set();
        wide_depot.templates[1].radius = Some(4.0);
        assert!(matches!(
            wide_depot.validate(),
            Err(TemplateError::RadiusOnNonUnit(id)) if id == "structure/depot"
        ));
    }

    #[test]
    fn kinds_know_whether_they_are_placeable() {
        assert!(TemplateKind::Unit.placeable());
        assert!(TemplateKind::Structure.placeable());
        assert!(TemplateKind::Prop.placeable());
        assert!(!TemplateKind::Faction.placeable());
    }

    #[test]
    fn kinds_know_whether_they_stamp_the_ground() {
        // A unit is placeable and does *not* stamp, which is the whole distinction: the two
        // predicates would be interchangeable without that row, and a moving footprint is the
        // mistake this refusal exists to catch.
        assert!(TemplateKind::Structure.stamps());
        assert!(TemplateKind::Prop.stamps());
        assert!(!TemplateKind::Unit.stamps());
        assert!(!TemplateKind::Faction.stamps());
    }

    #[test]
    fn a_stamped_template_round_trips_through_json() {
        let mut set = set();
        set.templates[1].footprint = Some(Footprint { cells: [3, 2] });
        set.templates[1].passage = Some(Passage {
            cells: [1, 6],
            class: 2,
        });
        let bytes = set.to_json().expect("serializes");
        let text = std::str::from_utf8(&bytes).expect("utf-8");
        assert!(
            text.contains("\"footprint\"") && text.contains("\"passage\""),
            "the stamps did not reach the document: {text}"
        );
        assert!(
            !text.contains("\"footprint\": null"),
            "an absent stamp must not be written at all: {text}"
        );
        assert_eq!(TemplateSet::from_json(&bytes).expect("parses"), set);
    }

    #[test]
    fn only_a_structure_or_a_prop_may_stamp_the_ground() {
        let with_footprint = |kind, model| {
            let mut entry = template("thing", kind, model);
            entry.footprint = Some(Footprint { cells: [2, 2] });
            if kind == TemplateKind::Unit {
                // Otherwise the unit is refused for the measurements it is missing and the
                // footprint — the thing under test — never gets looked at.
                entry.speed = Some(3.0);
                entry.radius = Some(1.5);
            }
            TemplateSet {
                format_version: 1,
                templates: vec![entry],
            }
        };
        assert!(
            with_footprint(TemplateKind::Structure, Some("m.glb"))
                .validate()
                .is_ok()
        );
        assert!(
            with_footprint(TemplateKind::Prop, Some("m.glb"))
                .validate()
                .is_ok()
        );
        assert!(matches!(
            with_footprint(TemplateKind::Unit, Some("m.glb")).validate(),
            Err(TemplateError::UnstampableKind(id)) if id == "thing"
        ));
        assert!(matches!(
            with_footprint(TemplateKind::Faction, None).validate(),
            Err(TemplateError::UnstampableKind(id)) if id == "thing"
        ));
    }

    #[test]
    fn an_empty_or_impassable_stamp_is_refused() {
        // Both refusals are about a file saying something it cannot mean. A zero extent covers no
        // ground, so it is a typo wearing the shape of a feature; and a passage of class zero
        // *denies* ground, which is what `footprint` is for and the opposite of what the word says.
        let stamped = |footprint, passage| {
            let mut entry = template("structure/bridge", TemplateKind::Structure, Some("b.glb"));
            entry.footprint = footprint;
            entry.passage = passage;
            TemplateSet {
                format_version: 1,
                templates: vec![entry],
            }
        };
        assert!(matches!(
            stamped(Some(Footprint { cells: [0, 3] }), None).validate(),
            Err(TemplateError::EmptyStamp(id)) if id == "structure/bridge"
        ));
        assert!(matches!(
            stamped(None, Some(Passage { cells: [4, 0], class: 2 })).validate(),
            Err(TemplateError::EmptyStamp(id)) if id == "structure/bridge"
        ));
        assert!(matches!(
            stamped(None, Some(Passage { cells: [4, 1], class: 0 })).validate(),
            Err(TemplateError::ImpassablePassage(id)) if id == "structure/bridge"
        ));
        assert!(
            stamped(
                Some(Footprint { cells: [1, 1] }),
                Some(Passage {
                    cells: [4, 1],
                    class: 1
                })
            )
            .validate()
            .is_ok(),
            "a template declaring both stamps is legal — a gatehouse is a bridge with a building \
             on it"
        );
    }
}
