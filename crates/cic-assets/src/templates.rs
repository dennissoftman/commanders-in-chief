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
//! A template today is an identifier, a kind, a model for the kinds that are drawn, and a display
//! name key. Health, speed, cost, weapons, and footprints arrive with the M6 mechanics that read
//! them — a field nothing consumes is a field nothing tests, which is the same reasoning that
//! deferred the whole format. Adding an optional field later does not break existing files; changing
//! what an existing field means takes a version bump.
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
    /// identifier, a placeable template with no model or a blank one, or a faction with a model.
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
        }
        Ok(())
    }

    /// The template with the given identifier.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Template> {
        self.templates.iter().find(|template| template.id == id)
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
    use super::{Template, TemplateError, TemplateKind, TemplateSet};

    fn template(id: &str, kind: TemplateKind, model: Option<&str>) -> Template {
        Template {
            id: id.to_owned(),
            kind,
            model: model.map(str::to_owned),
            name: None,
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
    fn kinds_know_whether_they_are_placeable() {
        assert!(TemplateKind::Unit.placeable());
        assert!(TemplateKind::Structure.placeable());
        assert!(TemplateKind::Prop.placeable());
        assert!(!TemplateKind::Faction.placeable());
    }
}
