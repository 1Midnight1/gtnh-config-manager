//! Small persisted app settings — currently just the last selected instance folder
//! (feature: remember the last folder across relaunches).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppSettings {
    pub last_instance: Option<PathBuf>,
}

impl AppSettings {
    fn path() -> Option<PathBuf> {
        let dirs = directories::ProjectDirs::from("", "", "gtnh-config-manager")?;
        Some(dirs.config_dir().join("settings.json"))
    }

    pub fn load() -> AppSettings {
        Self::load_from(Self::path().as_deref())
    }

    fn load_from(path: Option<&Path>) -> AppSettings {
        let Some(path) = path else { return AppSettings::default() };
        let Ok(contents) = std::fs::read_to_string(path) else { return AppSettings::default() };
        serde_json::from_str(&contents).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(Self::path().ok_or_else(|| std::io::Error::other("no config directory available"))?)
    }

    fn save_to(&self, path: PathBuf) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, contents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("gtnh-settings-test-{}", std::process::id()));
        let path = dir.join("settings.json");

        let settings = AppSettings { last_instance: Some(PathBuf::from("/some/instance")) };
        settings.save_to(path.clone()).unwrap();

        let loaded = AppSettings::load_from(Some(&path));
        assert_eq!(loaded, settings);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_file_yields_default() {
        assert_eq!(AppSettings::load_from(None), AppSettings::default());
    }
}
