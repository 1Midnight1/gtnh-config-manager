//! Lists and restores the world backups ServerUtilities writes to `.minecraft/backups` as
//! `yyyy-mm-dd-hh-mm-ss.zip` archives. Entries in those archives are paths relative to
//! `.minecraft` (e.g. `saves/<world>/level.dat`, `journeymap/data/sp/<world>/...`), and the zip
//! comment holds the world name.
//!
//! A restore replaces whole world-specific folders ("restore roots") rather than extracting over
//! existing files, so no newer chunks survive. Before anything is touched, the current contents
//! of those same roots are saved as a new backup in the same format, so a restore can itself be
//! undone by restoring that safety backup.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, BufReader, BufWriter};
use std::path::{Component, Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::config_store::{ConfigStore, PropertyPath};
use crate::forge_cfg::PropertyValue;

const DEFAULT_BACKUP_FOLDER: &str = "./backups/";
/// Scratch folder (inside `.minecraft`, so it's on the same filesystem and renames are cheap)
/// the selected backup is fully extracted into before any existing data is removed.
const STAGING_DIR: &str = ".gtnh-restore-staging";
const TIMESTAMP_FORMAT: &str = "%Y-%m-%d-%H-%M-%S";

#[derive(Debug, Clone, PartialEq)]
pub struct BackupInfo {
    pub path: PathBuf,
    /// File stem, i.e. the `yyyy-mm-dd-hh-mm-ss` timestamp.
    pub name: String,
    /// World name from the zip comment, if present.
    pub world: Option<String>,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct RestoreReport {
    /// The backup of the pre-restore state of every restored root.
    pub safety_backup: PathBuf,
    pub roots: Vec<PathBuf>,
}

/// Resolves ServerUtilities' configured backup folder (`backups/backup_folder_path` in
/// `serverutilities/serverutilities.cfg`), falling back to its default of `./backups/`.
pub fn backups_dir(store: &ConfigStore) -> PathBuf {
    let path = PropertyPath {
        relative_path: PathBuf::from("serverutilities/serverutilities.cfg"),
        category_path: vec!["backups".to_string()],
        property_name: "backup_folder_path".to_string(),
    };
    let configured = match store.get_property(&path).map(|property| &property.value) {
        Some(PropertyValue::Single(value)) if !value.trim().is_empty() => value.trim(),
        _ => DEFAULT_BACKUP_FOLDER,
    };
    store.minecraft_dir.join(configured)
}

/// Returns every `yyyy-mm-dd-hh-mm-ss.zip` file directly inside `dir`, newest first.
pub fn list_backups(dir: &Path) -> Vec<BackupInfo> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut backups: Vec<BackupInfo> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if !path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
            {
                return None;
            }
            let name = path.file_stem()?.to_str()?.to_string();
            if !is_backup_name(&name) {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            let world = File::open(&path)
                .ok()
                .and_then(|file| ZipArchive::new(BufReader::new(file)).ok())
                .and_then(|archive| world_from_comment(archive.comment()));
            Some(BackupInfo {
                path,
                name,
                world,
                size: metadata.len(),
            })
        })
        .collect();

    // The fixed-width timestamp format sorts chronologically as plain text.
    backups.sort_by(|a, b| b.name.cmp(&a.name));
    backups
}

/// Restores `backup` into `minecraft_dir`, first saving the current state of everything it's
/// about to replace as a new backup in `backups_dir`.
pub fn restore(
    minecraft_dir: &Path,
    backups_dir: &Path,
    backup: &BackupInfo,
) -> Result<RestoreReport, String> {
    let open = || -> Result<ZipArchive<BufReader<File>>, String> {
        let file = File::open(&backup.path).map_err(|err| format!("{}: {err}", backup.name))?;
        ZipArchive::new(BufReader::new(file)).map_err(|err| format!("{}: {err}", backup.name))
    };

    let mut archive = open()?;
    let (entries, prefix) = entry_paths(&mut archive, &backup.name)?;
    let world = world_from_comment(archive.comment())
        .or_else(|| world_from_entries(&entries))
        .ok_or_else(|| format!("{}: could not determine the world name", backup.name))?;
    let roots = restore_roots(&entries, &world);
    if roots.is_empty() {
        return Err(format!("{} contains no files", backup.name));
    }

    let safety_backup =
        create_backup(minecraft_dir, backups_dir, &roots, &world).map_err(|err| {
            format!("Failed to back up the current world, nothing was restored: {err}")
        })?;

    let staging = minecraft_dir.join(STAGING_DIR);
    let result = extract_to_staging(&mut archive, prefix.as_deref(), &staging)
        .map_err(|err| {
            format!(
                "Failed to extract {}, nothing was restored: {err}",
                backup.name
            )
        })
        .and_then(|()| {
            swap_in_roots(minecraft_dir, &staging, &roots).map_err(|err| {
                format!(
                    "Restore failed partway through ({err}); the previous state is saved in {}",
                    safety_backup.display()
                )
            })
        });
    let _ = std::fs::remove_dir_all(&staging);
    result?;

    Ok(RestoreReport {
        safety_backup,
        roots: roots.into_iter().collect(),
    })
}

fn is_backup_name(name: &str) -> bool {
    chrono::NaiveDateTime::parse_from_str(name, TIMESTAMP_FORMAT).is_ok() && name.len() == 19
}

fn world_from_comment(comment: &[u8]) -> Option<String> {
    let comment = std::str::from_utf8(comment).ok()?.trim();
    (!comment.is_empty()).then(|| comment.to_string())
}

/// Fallback for archives without a comment: the folder containing `saves/<world>/level.dat`.
fn world_from_entries(entries: &[PathBuf]) -> Option<String> {
    entries.iter().find_map(|entry| {
        let components: Vec<&str> = entry.iter().filter_map(|part| part.to_str()).collect();
        match components.as_slice() {
            ["saves", world, "level.dat"] => Some(world.to_string()),
            _ => None,
        }
    })
}

/// Returns every file entry's path relative to `.minecraft`, rejecting unsafe (absolute or `..`)
/// paths. If every entry sits under a `<backup name>/` folder, that prefix is stripped and
/// returned too, so extraction can strip it the same way.
fn entry_paths<R: io::Read + io::Seek>(
    archive: &mut ZipArchive<R>,
    backup_name: &str,
) -> Result<(Vec<PathBuf>, Option<PathBuf>), String> {
    let mut entries = Vec::new();
    for index in 0..archive.len() {
        let file = archive.by_index(index).map_err(|err| err.to_string())?;
        let path = file
            .enclosed_name()
            .ok_or_else(|| format!("unsafe path in archive: {}", file.name()))?;
        if !file.is_dir() {
            entries.push(path);
        }
    }

    let prefix = Path::new(backup_name);
    if !entries.is_empty() && entries.iter().all(|entry| entry.starts_with(prefix)) {
        let stripped = entries
            .iter()
            .map(|entry| entry.strip_prefix(prefix).unwrap().to_path_buf())
            .collect();
        return Ok((stripped, Some(prefix.to_path_buf())));
    }
    Ok((entries, None))
}

/// Maps each entry to the folder (or file) that gets replaced as a unit when restoring. That's
/// the shortest prefix whose last component is the world name or starts with `<world>_` (e.g.
/// `saves/<world>`, `visualprospecting/server/<world>_<uuid>`). Otherwise it's the entry's parent
/// folder (e.g. `saves/NEI/global`). A root is never a top-level `.minecraft` folder, so a world
/// named like one (e.g. `saves`) can't replace the whole folder. Roots nested inside another root
/// are dropped.
fn restore_roots(entries: &[PathBuf], world: &str) -> BTreeSet<PathBuf> {
    let world_prefix = format!("{world}_");
    let mut roots = BTreeSet::new();

    for entry in entries {
        let components: Vec<&std::ffi::OsStr> = entry
            .components()
            .filter_map(|component| match component {
                Component::Normal(part) => Some(part),
                _ => None,
            })
            .collect();
        let Some(last) = components.len().checked_sub(1) else {
            continue;
        };

        let world_index = (1..last).find(|&index| {
            components[index]
                .to_str()
                .is_some_and(|part| part == world || part.starts_with(&world_prefix))
        });
        let root_len = match world_index {
            Some(index) => index + 1,
            None if last >= 2 => last,
            None => last + 1,
        };
        roots.insert(components[..root_len].iter().collect::<PathBuf>());
    }

    let nested: Vec<PathBuf> = roots
        .iter()
        .filter(|root| {
            roots
                .iter()
                .any(|other| other != *root && root.starts_with(other))
        })
        .cloned()
        .collect();
    for root in nested {
        roots.remove(&root);
    }
    roots
}

/// Zips every existing file under `roots` (relative to `minecraft_dir`) into a new
/// `yyyy-mm-dd-hh-mm-ss.zip` in `backups_dir`, matching ServerUtilities' own layout.
fn create_backup(
    minecraft_dir: &Path,
    backups_dir: &Path,
    roots: &BTreeSet<PathBuf>,
    world: &str,
) -> io::Result<PathBuf> {
    std::fs::create_dir_all(backups_dir)?;

    let destination = loop {
        let name = chrono::Local::now().format(TIMESTAMP_FORMAT).to_string();
        let candidate = backups_dir.join(format!("{name}.zip"));
        if !candidate.exists() {
            break candidate;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    };
    let temporary = destination.with_extension("zip.tmp");

    let result = (|| -> io::Result<()> {
        let mut writer = ZipWriter::new(BufWriter::new(File::create(&temporary)?));
        writer.set_comment(world).map_err(io::Error::other)?;

        let mut files = Vec::new();
        let mut empty_dirs = Vec::new();
        for root in roots {
            collect_files(minecraft_dir, root, &mut files, &mut empty_dirs)?;
        }
        for relative_path in files {
            let absolute_path = minecraft_dir.join(&relative_path);
            let size = std::fs::metadata(&absolute_path)?.len();
            let options = SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .large_file(size >= u32::MAX as u64);
            writer
                .start_file(zip_name(&relative_path), options)
                .map_err(io::Error::other)?;
            io::copy(&mut File::open(&absolute_path)?, &mut writer)?;
        }
        // ServerUtilities' own backups have no directory entries, but recording empty folders
        // here makes restoring this safety backup reproduce the pre-restore state exactly.
        for relative_path in empty_dirs {
            writer
                .add_directory(zip_name(&relative_path), SimpleFileOptions::default())
                .map_err(io::Error::other)?;
        }

        writer.finish().map_err(io::Error::other)?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            std::fs::rename(&temporary, &destination)?;
            Ok(destination)
        }
        Err(err) => {
            let _ = std::fs::remove_file(&temporary);
            Err(err)
        }
    }
}

/// Appends every file at or under `relative_path` (relative to `minecraft_dir`) to `files`, and
/// every empty folder to `empty_dirs`. Missing paths are skipped - a root may not exist yet in
/// the current world.
fn collect_files(
    minecraft_dir: &Path,
    relative_path: &Path,
    files: &mut Vec<PathBuf>,
    empty_dirs: &mut Vec<PathBuf>,
) -> io::Result<()> {
    let absolute_path = minecraft_dir.join(relative_path);
    let Ok(metadata) = std::fs::symlink_metadata(&absolute_path) else {
        return Ok(());
    };
    if metadata.is_dir() {
        let mut children: Vec<_> = std::fs::read_dir(&absolute_path)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .collect();
        children.sort();
        if children.is_empty() {
            empty_dirs.push(relative_path.to_path_buf());
        }
        for child in children {
            collect_files(minecraft_dir, &relative_path.join(child), files, empty_dirs)?;
        }
    } else if metadata.is_file() {
        files.push(relative_path.to_path_buf());
    }
    Ok(())
}

fn zip_name(relative_path: &Path) -> String {
    relative_path
        .iter()
        .map(|part| part.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Extracts every entry into a freshly cleared `staging` folder, stripping `prefix`.
fn extract_to_staging<R: io::Read + io::Seek>(
    archive: &mut ZipArchive<R>,
    prefix: Option<&Path>,
    staging: &Path,
) -> io::Result<()> {
    if staging.exists() {
        std::fs::remove_dir_all(staging)?;
    }
    std::fs::create_dir_all(staging)?;

    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(io::Error::other)?;
        let Some(path) = file.enclosed_name() else {
            return Err(io::Error::other(format!(
                "unsafe path in archive: {}",
                file.name()
            )));
        };
        let relative_path = match prefix {
            Some(prefix) => match path.strip_prefix(prefix) {
                Ok(stripped) => stripped.to_path_buf(),
                // Only file entries decide the prefix; stray folder entries outside it are skipped.
                Err(_) if file.is_dir() => continue,
                Err(err) => return Err(io::Error::other(err)),
            },
            None => path,
        };
        let destination = staging.join(relative_path);
        if file.is_dir() {
            std::fs::create_dir_all(&destination)?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        io::copy(&mut file, &mut BufWriter::new(File::create(&destination)?))?;
    }
    Ok(())
}

/// Replaces each root under `minecraft_dir` with its staged copy.
fn swap_in_roots(
    minecraft_dir: &Path,
    staging: &Path,
    roots: &BTreeSet<PathBuf>,
) -> io::Result<()> {
    for root in roots {
        let target = minecraft_dir.join(root);
        match std::fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(&target)?,
            Ok(_) => std::fs::remove_file(&target)?,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(staging.join(root), &target)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use super::*;

    fn paths(entries: &[&str]) -> Vec<PathBuf> {
        entries.iter().map(PathBuf::from).collect()
    }

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gtnh-backups-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn zip_contents(path: &Path) -> (Option<String>, Vec<(String, String)>) {
        let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
        let world = world_from_comment(archive.comment());
        let mut files = Vec::new();
        for index in 0..archive.len() {
            let mut file = archive.by_index(index).unwrap();
            if file.is_dir() {
                continue;
            }
            let mut contents = String::new();
            file.read_to_string(&mut contents).unwrap();
            files.push((file.name().to_string(), contents));
        }
        files.sort();
        (world, files)
    }

    #[test]
    fn computes_restore_roots_like_serverutilities_backups() {
        let entries = paths(&[
            "saves/SolarGTNH/level.dat",
            "saves/SolarGTNH/DIM-1/region/r.0.0.mca",
            "saves/NEI/global/bookmarks.ini",
            "saves/NEI/local/SolarGTNH/NEI.dat",
            "journeymap/data/sp/SolarGTNH/DIM0/day/0,0.png",
            "TCNodeTracker/SolarGTNH/nodes.json",
            "visualprospecting/client/e25e/SolarGTNH_c983/DIM0.dat",
            "visualprospecting/server/SolarGTNH_c983/DIM0.dat",
        ]);

        let roots: Vec<PathBuf> = restore_roots(&entries, "SolarGTNH").into_iter().collect();

        assert_eq!(
            roots,
            paths(&[
                "TCNodeTracker/SolarGTNH",
                "journeymap/data/sp/SolarGTNH",
                "saves/NEI/global",
                "saves/NEI/local/SolarGTNH",
                "saves/SolarGTNH",
                "visualprospecting/client/e25e/SolarGTNH_c983",
                "visualprospecting/server/SolarGTNH_c983",
            ])
        );
    }

    #[test]
    fn never_uses_a_top_level_folder_as_a_root() {
        let entries = paths(&["saves/saves/level.dat", "journeymap/loose.dat", "top.txt"]);
        let roots: Vec<PathBuf> = restore_roots(&entries, "saves").into_iter().collect();
        assert_eq!(
            roots,
            paths(&["journeymap/loose.dat", "saves/saves", "top.txt"])
        );
    }

    #[test]
    fn resolves_the_configured_backups_folder() {
        let minecraft = test_dir("dir");
        assert_eq!(
            backups_dir(&ConfigStore::load(&minecraft).0),
            minecraft.join("./backups/")
        );

        write(
            &minecraft.join("serverutilities/serverutilities.cfg"),
            "backups {\n    S:backup_folder_path=./other-backups/\n}\n",
        );
        assert_eq!(
            backups_dir(&ConfigStore::load(&minecraft).0),
            minecraft.join("./other-backups/")
        );

        std::fs::remove_dir_all(&minecraft).unwrap();
    }

    #[test]
    fn recognizes_backup_names() {
        assert!(is_backup_name("2026-09-20-19-13-04"));
        assert!(!is_backup_name("2026-09-20"));
        assert!(!is_backup_name("World-20260912-203405"));
        assert!(!is_backup_name("2026-13-20-19-13-04"));
    }

    #[test]
    fn lists_backups_newest_first() {
        let dir = test_dir("list");
        for name in ["2026-09-12-14-22-28", "2026-09-20-19-13-04"] {
            let mut writer = ZipWriter::new(File::create(dir.join(format!("{name}.zip"))).unwrap());
            writer.set_comment("World").unwrap();
            writer.finish().unwrap();
        }
        write(&dir.join("not-a-backup.zip"), "");
        std::fs::create_dir_all(dir.join("2026-09-21-00-00-00")).unwrap();

        let backups = list_backups(&dir);
        let names: Vec<&str> = backups.iter().map(|backup| backup.name.as_str()).collect();
        assert_eq!(names, ["2026-09-20-19-13-04", "2026-09-12-14-22-28"]);
        assert_eq!(backups[0].world.as_deref(), Some("World"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_archives_with_unsafe_paths() {
        let dir = test_dir("unsafe");
        let path = dir.join("2026-01-01-00-00-00.zip");
        let mut writer = ZipWriter::new(File::create(&path).unwrap());
        writer
            .start_file("../escape.txt", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"nope").unwrap();
        writer.finish().unwrap();

        let mut archive = ZipArchive::new(File::open(&path).unwrap()).unwrap();
        assert!(entry_paths(&mut archive, "2026-01-01-00-00-00").is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn strips_a_wrapper_folder_named_after_the_backup() {
        let dir = test_dir("wrapper");
        let path = dir.join("2026-01-01-00-00-00.zip");
        let mut writer = ZipWriter::new(File::create(&path).unwrap());
        writer
            .start_file(
                "2026-01-01-00-00-00/saves/World/level.dat",
                SimpleFileOptions::default(),
            )
            .unwrap();
        writer.finish().unwrap();

        let mut archive = ZipArchive::new(File::open(&path).unwrap()).unwrap();
        let (entries, prefix) = entry_paths(&mut archive, "2026-01-01-00-00-00").unwrap();
        assert_eq!(entries, paths(&["saves/World/level.dat"]));
        assert_eq!(prefix, Some(PathBuf::from("2026-01-01-00-00-00")));
        assert_eq!(world_from_entries(&entries).as_deref(), Some("World"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn restore_replaces_roots_and_saves_the_previous_state() {
        let minecraft = test_dir("restore");
        let backups = minecraft.join("backups");
        let world = minecraft.join("saves/World");
        write(&world.join("level.dat"), "old level");
        write(&world.join("region/r.0.0.mca"), "old region");
        write(
            &minecraft.join("TCNodeTracker/World/nodes.json"),
            "old nodes",
        );
        write(&minecraft.join("saves/Other/level.dat"), "untouched");
        std::fs::create_dir_all(world.join("backpacks")).unwrap();

        let roots = BTreeSet::from([
            PathBuf::from("saves/World"),
            PathBuf::from("TCNodeTracker/World"),
        ]);
        let original = create_backup(&minecraft, &backups, &roots, "World").unwrap();
        let backup = list_backups(&backups).remove(0);
        assert_eq!(backup.path, original);

        // Play on after the backup: change a file and generate a new region.
        write(&world.join("level.dat"), "new level");
        write(&world.join("region/r.1.0.mca"), "new region");

        let report = restore(&minecraft, &backups, &backup).unwrap();

        assert_eq!(
            std::fs::read_to_string(world.join("level.dat")).unwrap(),
            "old level"
        );
        assert!(!world.join("region/r.1.0.mca").exists());
        assert_eq!(
            std::fs::read_to_string(minecraft.join("TCNodeTracker/World/nodes.json")).unwrap(),
            "old nodes"
        );
        assert_eq!(
            std::fs::read_to_string(minecraft.join("saves/Other/level.dat")).unwrap(),
            "untouched"
        );
        assert!(!minecraft.join(STAGING_DIR).exists());
        assert!(world.join("backpacks").is_dir());

        let (safety_world, safety_files) = zip_contents(&report.safety_backup);
        assert_eq!(safety_world.as_deref(), Some("World"));
        assert_eq!(
            safety_files,
            [
                ("TCNodeTracker/World/nodes.json", "old nodes"),
                ("saves/World/level.dat", "new level"),
                ("saves/World/region/r.0.0.mca", "old region"),
                ("saves/World/region/r.1.0.mca", "new region"),
            ]
            .map(|(name, contents)| (name.to_string(), contents.to_string()))
        );

        std::fs::remove_dir_all(&minecraft).unwrap();
    }
}
