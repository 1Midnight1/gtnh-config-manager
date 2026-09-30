//! Whole files (not Forge `.cfg`s) whose contents are stored verbatim in each profile - e.g.
//! ServerUtilities' `ranks.txt`, which is edited through an in-game GUI rather than here.
//! Captured from disk when a profile is saved and written back when a profile is selected.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::config_store::FileError;
use crate::fsutil;

/// ServerUtilities' rank definitions, shown on the Profiles page.
pub const RANKS_FILE: &str = "serverutilities/server/ranks.txt";

/// Paths (relative to `.minecraft`) stored in every profile.
pub const TRACKED_FILES: &[&str] = &[RANKS_FILE];

/// Reads every tracked file that currently exists under `minecraft_dir`.
pub fn capture(minecraft_dir: &Path) -> BTreeMap<PathBuf, String> {
    TRACKED_FILES
        .iter()
        .map(PathBuf::from)
        .filter_map(|relative_path| {
            let contents = std::fs::read_to_string(minecraft_dir.join(&relative_path)).ok()?;
            Some((relative_path, contents))
        })
        .collect()
}

/// Reads ranks.txt under `minecraft_dir`, or `None` if it doesn't exist or can't be read.
pub fn read_ranks(minecraft_dir: &Path) -> Option<String> {
    std::fs::read_to_string(minecraft_dir.join(RANKS_FILE)).ok()
}

/// Writes each stored file back under `minecraft_dir`, creating parent directories as needed.
pub fn write_all(minecraft_dir: &Path, files: &BTreeMap<PathBuf, String>) -> Vec<FileError> {
    let mut errors = Vec::new();
    for (relative_path, contents) in files {
        if let Err(err) =
            fsutil::write_atomic(&minecraft_dir.join(relative_path), contents.as_bytes())
        {
            errors.push(FileError {
                relative_path: relative_path.clone(),
                message: err.to_string(),
            });
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_and_writes_back_tracked_files() {
        let dir = std::env::temp_dir().join(format!("gtnh-tracked-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        assert!(capture(&dir).is_empty());
        assert!(read_ranks(&dir).is_none());

        let ranks = dir.join("serverutilities/server/ranks.txt");
        std::fs::create_dir_all(ranks.parent().unwrap()).unwrap();
        std::fs::write(&ranks, "[player]\npower: 1\n").unwrap();

        let captured = capture(&dir);
        assert_eq!(
            captured.get(Path::new("serverutilities/server/ranks.txt")),
            Some(&"[player]\npower: 1\n".to_string())
        );

        std::fs::remove_dir_all(&dir).unwrap();
        assert!(write_all(&dir, &captured).is_empty());
        assert_eq!(
            std::fs::read_to_string(&ranks).unwrap(),
            "[player]\npower: 1\n"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
