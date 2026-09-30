//! Manages named, persisted profiles (each wrapping a `Changeset`) so the program - not the
//! user - is responsible for where they live on disk. Mirrors `settings.rs`'s load/save pattern.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::changeset::Changeset;
use crate::fsutil;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub changeset: Changeset,
    /// Verbatim contents of whole files (see `tracked_files`), keyed by path relative to
    /// `.minecraft`. Defaulted so profiles saved before this field existed still load.
    #[serde(default)]
    pub tracked_files: BTreeMap<PathBuf, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfileStore {
    pub profiles: Vec<Profile>,
}

impl ProfileStore {
    fn path() -> Option<PathBuf> {
        fsutil::app_config_path("profiles.json")
    }

    /// Loads the saved profiles, plus a warning if the file existed but couldn't be read.
    pub fn load() -> (ProfileStore, Option<String>) {
        match Self::path() {
            Some(path) => Self::load_from(&path),
            None => (ProfileStore::default(), None),
        }
    }

    /// A damaged file is moved aside (to `profiles.json.corrupt-<timestamp>`) rather than left
    /// in place, so the next save can't overwrite the only copy of the user's profiles.
    fn load_from(path: &Path) -> (ProfileStore, Option<String>) {
        match fsutil::load_json(path) {
            Ok(store) => (store.unwrap_or_default(), None),
            Err(err) => {
                let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
                let aside = path.with_extension(format!("json.corrupt-{timestamp}"));
                let warning = match std::fs::rename(path, &aside) {
                    Ok(()) => format!(
                        "Couldn't read saved profiles ({err}); the file was moved to {}",
                        aside.display()
                    ),
                    Err(rename_err) => format!(
                        "Couldn't read saved profiles ({err}) or move the file aside \
                         ({rename_err}); saving will overwrite {}",
                        path.display()
                    ),
                };
                (ProfileStore::default(), Some(warning))
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path =
            Self::path().ok_or_else(|| std::io::Error::other("no config directory available"))?;
        self.save_to(&path)
    }

    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        fsutil::save_json(path, self)
    }

    /// Creates a new profile, or overwrites the changeset of an existing one with the same name
    /// (leaving its tracked files untouched).
    pub fn upsert(&mut self, name: String, changeset: Changeset) {
        if let Some(existing) = self
            .profiles
            .iter_mut()
            .find(|profile| profile.name == name)
        {
            existing.changeset = changeset;
        } else {
            self.profiles.push(Profile {
                name,
                changeset,
                tracked_files: BTreeMap::new(),
            });
        }
    }

    /// Replaces the stored tracked files of an existing profile. Does nothing if `name` doesn't
    /// exist.
    pub fn set_tracked_files(&mut self, name: &str, files: BTreeMap<PathBuf, String>) {
        if let Some(profile) = self
            .profiles
            .iter_mut()
            .find(|profile| profile.name == name)
        {
            profile.tracked_files = files;
        }
    }

    pub fn remove(&mut self, name: &str) {
        self.profiles.retain(|profile| profile.name != name);
    }

    pub fn get(&self, name: &str) -> Option<&Profile> {
        self.profiles.iter().find(|profile| profile.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("gtnh-profiles-test-{}", std::process::id()));
        let path = dir.join("profiles.json");

        let mut store = ProfileStore::default();
        store.upsert("My Profile".to_string(), Changeset::default());
        store.save_to(&path).unwrap();

        assert_eq!(ProfileStore::load_from(&path), (store, None));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn moves_a_damaged_file_aside_instead_of_loading_it_as_empty() {
        let dir = std::env::temp_dir().join(format!("gtnh-profiles-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("profiles.json");
        std::fs::write(&path, "{\"profiles\": [trunc").unwrap();

        let (store, warning) = ProfileStore::load_from(&path);
        assert_eq!(store, ProfileStore::default());
        assert!(warning.is_some());
        assert!(!path.exists());
        let moved: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert_eq!(moved.len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn upsert_overwrites_existing_profile_with_same_name() {
        let mut store = ProfileStore::default();
        store.upsert("A".to_string(), Changeset::default());
        assert_eq!(store.profiles.len(), 1);

        let mut second = Changeset::default();
        second.record(
            crate::config_store::PropertyPath {
                relative_path: "thing.cfg".into(),
                category_path: vec!["modules".to_string()],
                property_name: "Flag".to_string(),
            },
            crate::forge_cfg::PropertyValue::Single("true".to_string()),
            crate::forge_cfg::PropertyValue::Single("false".to_string()),
        );
        store.upsert("A".to_string(), second.clone());

        assert_eq!(store.profiles.len(), 1);
        assert_eq!(store.get("A").unwrap().changeset, second);
    }

    #[test]
    fn upsert_keeps_tracked_files() {
        let mut store = ProfileStore::default();
        store.upsert("A".to_string(), Changeset::default());
        let files = BTreeMap::from([(PathBuf::from("ranks.txt"), "contents".to_string())]);
        store.set_tracked_files("A", files.clone());
        store.upsert("A".to_string(), Changeset::default());
        assert_eq!(store.get("A").unwrap().tracked_files, files);
    }

    #[test]
    fn loads_profiles_saved_without_tracked_files() {
        let json = r#"{"profiles":[{"name":"Old","changeset":{"entries":[]}}]}"#;
        let store: ProfileStore = serde_json::from_str(json).unwrap();
        assert!(store.get("Old").unwrap().tracked_files.is_empty());
    }

    #[test]
    fn remove_deletes_by_name() {
        let mut store = ProfileStore::default();
        store.upsert("A".to_string(), Changeset::default());
        store.upsert("B".to_string(), Changeset::default());
        store.remove("A");
        assert!(store.get("A").is_none());
        assert!(store.get("B").is_some());
    }
}
