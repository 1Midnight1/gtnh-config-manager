//! Small persisted app settings: the last selected instance folder and the active profile, so
//! both are restored on the next launch.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub last_instance: Option<PathBuf>,
    pub active_profile: Option<String>,
}

impl AppSettings {
    fn path() -> Option<PathBuf> {
        fsutil::app_config_path("settings.json")
    }

    /// Loads the saved settings. Nothing in them is irreplaceable, so a missing or damaged file
    /// just yields the defaults.
    pub fn load() -> AppSettings {
        Self::path()
            .map(|path| Self::load_from(&path))
            .unwrap_or_default()
    }

    fn load_from(path: &Path) -> AppSettings {
        fsutil::load_json(path).ok().flatten().unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path =
            Self::path().ok_or_else(|| std::io::Error::other("no config directory available"))?;
        self.save_to(&path)
    }

    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        fsutil::save_json(path, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("gtnh-settings-test-{}", std::process::id()));
        let path = dir.join("settings.json");

        let settings = AppSettings {
            last_instance: Some(PathBuf::from("/some/instance")),
            active_profile: Some("Server".to_string()),
        };
        settings.save_to(&path).unwrap();
        assert_eq!(AppSettings::load_from(&path), settings);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_fields_and_files_yield_defaults() {
        let dir = std::env::temp_dir().join(format!("gtnh-settings-old-{}", std::process::id()));
        let path = dir.join("settings.json");
        assert_eq!(AppSettings::load_from(&path), AppSettings::default());

        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, r#"{"last_instance": "/old"}"#).unwrap();
        assert_eq!(
            AppSettings::load_from(&path),
            AppSettings {
                last_instance: Some(PathBuf::from("/old")),
                active_profile: None,
            }
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
