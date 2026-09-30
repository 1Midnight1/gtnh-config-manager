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
    pub fn record(
        &mut self,
        path: PropertyPath,
        original_value: PropertyValue,
        new_value: PropertyValue,
    ) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.path == path) {
            entry.new_value = new_value;
        } else {
            self.entries.push(ChangesetEntry {
                path,
                original_value,
                new_value,
            });
        }
    }

    /// Applies every recorded edit onto `store`. Returns the paths that no longer resolved
    /// to an existing property of the same shape (scalar or list), so the caller can surface
    /// them instead of failing silently.
    ///
    /// Where the store holds something other than `new_value`, that value becomes the entry's
    /// `original_value`: it's what's really there (e.g. after a modpack update, or in another
    /// instance), so it's what `revert` must restore.
    pub fn apply(&mut self, store: &mut ConfigStore) -> Vec<PropertyPath> {
        let mut unresolved = Vec::new();
        for entry in &mut self.entries {
            let current = match store.get_property(&entry.path) {
                Some(property) if property.value.same_shape(&entry.new_value) => &property.value,
                _ => {
                    unresolved.push(entry.path.clone());
                    continue;
                }
            };
            if *current != entry.new_value {
                entry.original_value = current.clone();
                store.set_property_value(&entry.path, entry.new_value.clone());
            }
        }
        unresolved
    }

    /// Reverts every recorded edit back to its `original_value`, undoing this changeset's
    /// effect on `store`. Returns the paths that no longer resolved to an existing property.
    pub fn revert(&self, store: &mut ConfigStore) -> Vec<PropertyPath> {
        let mut unresolved = Vec::new();
        for entry in &self.entries {
            let same_shape = store
                .get_property(&entry.path)
                .is_some_and(|property| property.value.same_shape(&entry.original_value));
            if same_shape {
                store.set_property_value(&entry.path, entry.original_value.clone());
            } else {
                unresolved.push(entry.path.clone());
            }
        }
        unresolved
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config_store::ConfigFileEntry;

    fn store_with(cfg: &str) -> ConfigStore {
        ConfigStore {
            minecraft_dir: PathBuf::new(),
            files: vec![
                ConfigFileEntry::parse(PathBuf::from("thing.cfg"), cfg.to_string()).unwrap(),
            ],
        }
    }

    #[test]
    fn apply_only_touches_recorded_paths_and_skips_missing_ones() {
        let mut store = store_with("modules {\n    B:Flag=true\n    B:Other=true\n}\n");

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
        assert!(store.has_unsaved_changes());

        let flag = store.get_property(&PropertyPath {
            relative_path: PathBuf::from("thing.cfg"),
            category_path: vec!["modules".to_string()],
            property_name: "Flag".to_string(),
        });
        assert_eq!(
            flag.unwrap().value,
            PropertyValue::Single("false".to_string())
        );

        let other = store.get_property(&PropertyPath {
            relative_path: PathBuf::from("thing.cfg"),
            category_path: vec!["modules".to_string()],
            property_name: "Other".to_string(),
        });
        assert_eq!(
            other.unwrap().value,
            PropertyValue::Single("true".to_string())
        );
    }

    #[test]
    fn revert_restores_original_values() {
        let mut store = store_with("modules {\n    B:Flag=true\n}\n");
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
        assert_eq!(
            store.get_property(&path).unwrap().value,
            PropertyValue::Single("false".to_string())
        );

        let unresolved = changeset.revert(&mut store);
        assert!(unresolved.is_empty());
        assert_eq!(
            store.get_property(&path).unwrap().value,
            PropertyValue::Single("true".to_string())
        );
    }

    #[test]
    fn apply_rebases_originals_onto_what_is_actually_there() {
        let mut store =
            store_with("modules {\n    I:Count=5\n    I:Same=2\n    I:L <\n     >\n}\n");
        let path = |name: &str| PropertyPath {
            relative_path: PathBuf::from("thing.cfg"),
            category_path: vec!["modules".to_string()],
            property_name: name.to_string(),
        };
        let single = |value: &str| PropertyValue::Single(value.to_string());

        let mut changeset = Changeset::default();
        // Recorded against an older version of the file, where Count was 1.
        changeset.record(path("Count"), single("1"), single("9"));
        // Already applied on disk: the stored original is the only record of the old value.
        changeset.record(path("Same"), single("0"), single("2"));
        // The property became a list since this was recorded.
        changeset.record(path("L"), single("a"), single("b"));

        let unresolved = changeset.apply(&mut store);
        assert_eq!(unresolved, [path("L")]);
        assert_eq!(changeset.entries[0].original_value, single("5"));
        assert_eq!(changeset.entries[1].original_value, single("0"));

        changeset.revert(&mut store);
        assert_eq!(
            store.get_property(&path("Count")).unwrap().value,
            single("5")
        );
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
        assert_eq!(
            changeset.entries[0].original_value,
            PropertyValue::Single("true".to_string())
        );
        assert_eq!(
            changeset.entries[0].new_value,
            PropertyValue::Single("maybe".to_string())
        );
    }
}
