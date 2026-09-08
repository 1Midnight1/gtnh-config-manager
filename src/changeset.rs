//! Tracks edits made through this program as a diff (property path -> new value), separate
//! from the full parsed config files. Applying a changeset to a freshly loaded `ConfigStore`
//! only touches the recorded paths and silently skips any that no longer resolve - so it never
//! clobbers configs that changed on disk (e.g. via a modpack update) but weren't edited here.

use serde::{Deserialize, Serialize};

use crate::config_store::{ConfigStore, PropertyPath};
use crate::forge_cfg::PropertyValue;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangesetEntry {
    pub path: PropertyPath,
    /// The property's value before this changeset first touched it, kept so the change can
    /// be shown as a diff (original -> new) without needing to re-read the on-disk file.
    pub original_value: PropertyValue,
    pub new_value: PropertyValue,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Changeset {
    pub entries: Vec<ChangesetEntry>,
}

impl Changeset {
    /// Records an edit. If this property was already recorded, only its `new_value` is
    /// updated - `original_value` always reflects the value from before *any* edit in this
    /// changeset, not just the most recent one.
    pub fn record(&mut self, path: PropertyPath, original_value: PropertyValue, new_value: PropertyValue) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.path == path) {
            entry.new_value = new_value;
        } else {
            self.entries.push(ChangesetEntry { path, original_value, new_value });
        }
    }

    /// Applies every recorded edit onto `store`. Returns the paths that no longer resolved
    /// to an existing property, so the caller can surface them instead of failing silently.
    pub fn apply(&self, store: &mut ConfigStore) -> Vec<PropertyPath> {
        let mut unresolved = Vec::new();
        for entry in &self.entries {
            if !store.set_property_value(&entry.path, entry.new_value.clone()) {
                unresolved.push(entry.path.clone());
            }
        }
        unresolved
    }

    /// Reverts every recorded edit back to its `original_value`, undoing this changeset's
    /// effect on `store`. Returns the paths that no longer resolved to an existing property.
    pub fn revert(&self, store: &mut ConfigStore) -> Vec<PropertyPath> {
        let mut unresolved = Vec::new();
        for entry in &self.entries {
            if !store.set_property_value(&entry.path, entry.original_value.clone()) {
                unresolved.push(entry.path.clone());
            }
        }
        unresolved
    }

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config_store::ConfigFileEntry;
    use crate::forge_cfg;

    #[test]
    fn apply_only_touches_recorded_paths_and_skips_missing_ones() {
        let ast = forge_cfg::parse("modules {\n    B:Flag=true\n    B:Other=true\n}\n").unwrap();
        let mut store = ConfigStore {
            minecraft_dir: PathBuf::new(),
            files: vec![ConfigFileEntry { relative_path: PathBuf::from("thing.cfg"), ast, dirty: false }],
        };

        let mut changeset = Changeset::default();
        changeset.record(
            PropertyPath {
                relative_path: PathBuf::from("thing.cfg"),
                category_path: vec!["modules".to_string()],
                property_name: "Flag".to_string(),
            },
            PropertyValue::Single("true".to_string()),
            PropertyValue::Single("false".to_string()),
        );
        changeset.record(
            PropertyPath {
                relative_path: PathBuf::from("thing.cfg"),
                category_path: vec!["modules".to_string()],
                property_name: "Removed".to_string(),
            },
            PropertyValue::Single("true".to_string()),
            PropertyValue::Single("false".to_string()),
        );

        let unresolved = changeset.apply(&mut store);
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].property_name, "Removed");

        let flag = store.get_property(&PropertyPath {
            relative_path: PathBuf::from("thing.cfg"),
            category_path: vec!["modules".to_string()],
            property_name: "Flag".to_string(),
        });
        assert_eq!(flag.unwrap().value, PropertyValue::Single("false".to_string()));

        let other = store.get_property(&PropertyPath {
            relative_path: PathBuf::from("thing.cfg"),
            category_path: vec!["modules".to_string()],
            property_name: "Other".to_string(),
        });
        assert_eq!(other.unwrap().value, PropertyValue::Single("true".to_string()));
    }

    #[test]
    fn revert_restores_original_values() {
        let ast = forge_cfg::parse("modules {\n    B:Flag=true\n}\n").unwrap();
        let mut store = ConfigStore {
            minecraft_dir: PathBuf::new(),
            files: vec![ConfigFileEntry { relative_path: PathBuf::from("thing.cfg"), ast, dirty: false }],
        };
        let path = PropertyPath {
            relative_path: PathBuf::from("thing.cfg"),
            category_path: vec!["modules".to_string()],
            property_name: "Flag".to_string(),
        };

        let mut changeset = Changeset::default();
        changeset.record(
            path.clone(),
            PropertyValue::Single("true".to_string()),
            PropertyValue::Single("false".to_string()),
        );
        changeset.apply(&mut store);
        assert_eq!(store.get_property(&path).unwrap().value, PropertyValue::Single("false".to_string()));

        let unresolved = changeset.revert(&mut store);
        assert!(unresolved.is_empty());
        assert_eq!(store.get_property(&path).unwrap().value, PropertyValue::Single("true".to_string()));
    }

    #[test]
    fn round_trips_through_json() {
        let mut changeset = Changeset::default();
        changeset.record(
            PropertyPath {
                relative_path: PathBuf::from("thing.cfg"),
                category_path: vec!["modules".to_string()],
                property_name: "Flag".to_string(),
            },
            PropertyValue::Single("true".to_string()),
            PropertyValue::Single("false".to_string()),
        );

        let json = changeset.to_json().unwrap();
        let reparsed = Changeset::from_json(&json).unwrap();
        assert_eq!(changeset, reparsed);
    }

    #[test]
    fn original_value_is_preserved_across_repeated_edits() {
        let path = PropertyPath {
            relative_path: PathBuf::from("thing.cfg"),
            category_path: vec!["modules".to_string()],
            property_name: "Flag".to_string(),
        };

        let mut changeset = Changeset::default();
        changeset.record(
            path.clone(),
            PropertyValue::Single("true".to_string()),
            PropertyValue::Single("false".to_string()),
        );
        // A second edit of the same property must not overwrite the original recorded value.
        changeset.record(
            path.clone(),
            PropertyValue::Single("false".to_string()),
            PropertyValue::Single("maybe".to_string()),
        );

        assert_eq!(changeset.entries.len(), 1);
        assert_eq!(changeset.entries[0].original_value, PropertyValue::Single("true".to_string()));
        assert_eq!(changeset.entries[0].new_value, PropertyValue::Single("maybe".to_string()));
    }
}
