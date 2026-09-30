//! Crash-safe file writing and the JSON persistence shared by `settings` and `profiles`.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Path of `file_name` in this program's own config directory (e.g.
/// `~/.config/gtnh-config-manager/`), or `None` if the OS doesn't provide one.
pub fn app_config_path(file_name: &str) -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "gtnh-config-manager")?;
    Some(dirs.config_dir().join(file_name))
}

/// Writes `contents` to `path` so that a crash or full disk never leaves a truncated file: the
/// data goes to a temporary file next to it, is synced, and then renamed over `path`. Missing
/// parent folders are created.
pub fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut temporary_name = path.file_name().unwrap_or_default().to_os_string();
    temporary_name.push(".tmp");
    let temporary = path.with_file_name(temporary_name);

    let result = File::create(&temporary)
        .and_then(|mut file| {
            file.write_all(contents)?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Reads and parses a JSON file. A missing file is `Ok(None)`; any other failure (unreadable or
/// unparseable) is an error, so callers never mistake a damaged file for an empty one.
pub fn load_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.to_string()),
    };
    serde_json::from_str(&contents)
        .map(Some)
        .map_err(|err| err.to_string())
}

pub fn save_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let contents = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    write_atomic(path, contents.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_atomically_and_loads_json() {
        let dir = std::env::temp_dir().join(format!("gtnh-fsutil-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("nested/data.json");

        assert_eq!(load_json::<Vec<u32>>(&path), Ok(None));
        save_json(&path, &vec![1, 2, 3]).unwrap();
        assert_eq!(load_json::<Vec<u32>>(&path), Ok(Some(vec![1, 2, 3])));
        write_atomic(&path, b"[4]").unwrap();
        assert_eq!(load_json::<Vec<u32>>(&path), Ok(Some(vec![4])));
        assert!(!dir.join("nested/data.json.tmp").exists());

        fs::write(&path, "not json").unwrap();
        assert!(load_json::<Vec<u32>>(&path).is_err());

        fs::remove_dir_all(&dir).unwrap();
    }
}
