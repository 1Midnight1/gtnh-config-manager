//! Manages named, persisted profiles (each wrapping a `Changeset`) so the program - not the
//! user - is responsible for where they live on disk. Mirrors `settings.rs`'s load/save pattern.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::changeset::Changeset;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub changeset: Changeset,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfileStore {
    pub profiles: Vec<Profile>,
}

impl ProfileStore {
    fn path() -> Option<PathBuf> {
        let dirs = directories::ProjectDirs::from("", "", "gtnh-config-manager")?;
        Some(dirs.config_dir().join("profiles.json"))
    }

    pub fn load() -> ProfileStore {
        Self::load_from(Self::path().as_deref())
    }

    fn load_from(path: Option<&Path>) -> ProfileStore {
        let Some(path) = path else {
            return ProfileStore::default();
        };
        let Ok(contents) = std::fs::read_to_string(path) else {
            return ProfileStore::default();
        };
        serde_json::from_str(&contents).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(
            Self::path().ok_or_else(|| std::io::Error::other("no config directory available"))?,
        )
    }

    fn save_to(&self, path: PathBuf) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, contents)
    }

    /// Creates a new profile, or overwrites the changeset of an existing one with the same name.
    pub fn upsert(&mut self, name: String, changeset: Changeset) {
        if let Some(existing) = self
            .profiles
            .iter_mut()
            .find(|profile| profile.name == name)
        {
            existing.changeset = changeset;
        } else {
            self.profiles.push(Profile { name, changeset });
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
        store.save_to(path.clone()).unwrap();

        let loaded = ProfileStore::load_from(Some(&path));
        assert_eq!(loaded, store);

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
    fn remove_deletes_by_name() {
        let mut store = ProfileStore::default();
        store.upsert("A".to_string(), Changeset::default());
        store.upsert("B".to_string(), Changeset::default());
        store.remove("A");
        assert!(store.get("A").is_none());
        assert!(store.get("B").is_some());
    }
}
