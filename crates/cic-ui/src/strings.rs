//! Display text, kept out of layout files and behind a key.
//!
//! # Why this exists before there is a second language
//!
//! Translating is content work and nobody is asking for it yet. *Making it possible* is structural
//! work, and it is the kind that cannot be retrofitted cheaply: literal strings spread through every
//! layout file, every widget default, and every error path, and finding them all afterwards is a
//! search for text that looks exactly like text which must not be translated. Starting with a key
//! costs one indirection now and removes that search entirely.
//!
//! # Why a missing key is not an empty string
//!
//! Falling back to `""` makes a typo invisible: the button is still there, still clickable, and
//! simply has no label, which reads as a rendering bug. Falling back to the key itself is the other
//! common choice and is better, because `menu.start_game` on screen names its own fix. This does both
//! — [`StringTable::get`] reports the absence so a loader can refuse, and [`StringTable::text`]
//! yields the key for a caller that must render something regardless.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Text for one language, keyed by the identifiers layout files use.
///
/// A `BTreeMap` rather than a `HashMap` because the resource layer's determinism rule reaches
/// anything that can affect load order or reported ordering, and a sorted map also makes a serialised
/// table diffable — which is the same argument the scenario format makes for JSON.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StringTable {
    entries: BTreeMap<String, String>,
}

impl StringTable {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes a table from JSON bytes.
    ///
    /// Takes bytes rather than a path, because nothing above the resource layer opens a file.
    ///
    /// # Errors
    ///
    /// Returns the `serde_json` error when the bytes are not a JSON object of strings.
    pub fn from_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    /// Inserts or replaces one entry, returning the previous text if there was any.
    pub fn set(&mut self, key: impl Into<String>, text: impl Into<String>) -> Option<String> {
        self.entries.insert(key.into(), text.into())
    }

    /// Looks up a key, reporting absence.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    /// Looks up a key, falling back to the key itself.
    ///
    /// For a caller that has to put *something* on screen. The key is deliberately visible rather
    /// than blank, so a missing entry looks like the mistake it is instead of like a layout fault.
    #[must_use]
    pub fn text<'a>(&'a self, key: &'a str) -> &'a str {
        self.get(key).unwrap_or(key)
    }

    /// How many entries there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every key present, in sorted order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// Returns the keys named by `wanted` that this table does not define, in sorted order.
    ///
    /// The check a loader runs against a layout, so a missing label is a load-time error naming every
    /// key at once rather than a blank control found by a player. Sorted and deduplicated because an
    /// error message that changes order between runs is one nobody can diff.
    #[must_use]
    pub fn missing<'a, I>(&self, wanted: I) -> Vec<&'a str>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let mut absent: Vec<&str> = wanted
            .into_iter()
            .filter(|key| self.get(key).is_none())
            .collect();
        absent.sort_unstable();
        absent.dedup();
        absent
    }
}

#[cfg(test)]
mod tests {
    use super::StringTable;

    fn table() -> StringTable {
        let mut table = StringTable::new();
        table.set("menu.play", "Play");
        table.set("menu.quit", "Quit");
        table
    }

    #[test]
    fn a_present_key_resolves_and_an_absent_one_is_reported() {
        let table = table();
        assert_eq!(table.get("menu.play"), Some("Play"));
        assert_eq!(table.get("menu.missing"), None);
        assert_eq!(table.len(), 2);
        assert!(!table.is_empty());
    }

    #[test]
    fn an_absent_key_renders_as_itself_rather_than_as_nothing() {
        // A blank button reads as a rendering bug; a button saying `menu.absent` names its own fix.
        let table = table();
        assert_eq!(table.text("menu.play"), "Play");
        assert_eq!(table.text("menu.absent"), "menu.absent");
    }

    #[test]
    fn missing_keys_come_back_sorted_and_deduplicated() {
        // A loader puts these straight into an error message, so the order cannot depend on the order
        // the layout happened to mention them in.
        let table = table();
        let absent = table.missing(["menu.quit", "z.late", "a.early", "z.late", "menu.play"]);
        assert_eq!(absent, vec!["a.early", "z.late"]);
    }

    #[test]
    fn a_table_is_json_and_decodes_from_bytes() {
        let decoded =
            StringTable::from_json(br#"{"menu.play":"Play","menu.quit":"Quit"}"#).expect("decode");
        assert_eq!(decoded, table());
        // Sorted on the way out, so a committed table stays diffable.
        let encoded = serde_json::to_string(&decoded).expect("encode");
        assert_eq!(encoded, r#"{"menu.play":"Play","menu.quit":"Quit"}"#);
    }

    #[test]
    fn keys_are_reported_in_sorted_order() {
        let mut table = StringTable::new();
        table.set("z", "last");
        table.set("a", "first");
        assert_eq!(table.keys().collect::<Vec<_>>(), vec!["a", "z"]);
    }
}
