//! In-memory model of a modpack's config files: discovers `.cfg` files under `.minecraft/config`
//! and `.minecraft/serverutilities`, parses them via `forge_cfg`, and allows looking up /
//! mutating individual properties by a stable `PropertyPath` that survives across reloads.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::forge_cfg::{self, ConfigFile, Item, Property, PropertyValue};

/// Top-level directories (relative to `.minecraft`) scanned for `.cfg` files, each up to one
/// subdirectory deep.
const SCAN_ROOTS: &[&str] = &["config", "serverutilities"];

/// Identifies a single property independent of its position in the AST, so it stays valid
/// across reloads and can be used as a stable key in search results and changesets.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PropertyPath {
    /// Path of the containing file, relative to the `.minecraft` directory (e.g.
    /// `config/GTNewHorizons/dreamcraft.cfg` or `serverutilities/foo.cfg`).
    pub relative_path: PathBuf,
    pub category_path: Vec<String>,
    pub property_name: String,
}

#[derive(Debug, Clone)]
pub struct ConfigFileEntry {
    pub relative_path: PathBuf,
    pub ast: ConfigFile,
    pub dirty: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ConfigStore {
    /// The instance's `.minecraft` directory; all `relative_path`s are relative to this.
    pub minecraft_dir: PathBuf,
    pub files: Vec<ConfigFileEntry>,
}

#[derive(Debug, Clone)]
pub struct LoadError {
    pub relative_path: PathBuf,
    pub message: String,
}

impl ConfigStore {
    /// Discovers `.cfg` files under each of `SCAN_ROOTS` inside `minecraft_dir` (each root
    /// scanned up to one subdirectory deep), then parses each of them. A single file failing
    /// to parse doesn't abort the whole load; its error is returned alongside the store built
    /// from the remaining files.
    pub fn load(minecraft_dir: &Path) -> (ConfigStore, Vec<LoadError>) {
        let mut files = Vec::new();
        let mut errors = Vec::new();

        for relative_path in discover_cfg_files(minecraft_dir) {
            let absolute_path = minecraft_dir.join(&relative_path);
            match std::fs::read_to_string(&absolute_path).map_err(|err| err.to_string()) {
                Ok(contents) => match forge_cfg::parse(&contents) {
                    Ok(ast) => files.push(ConfigFileEntry { relative_path, ast, dirty: false }),
                    Err(err) => errors.push(LoadError { relative_path, message: err.to_string() }),
                },
                Err(message) => errors.push(LoadError { relative_path, message }),
            }
        }

        files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

        (ConfigStore { minecraft_dir: minecraft_dir.to_path_buf(), files }, errors)
    }

    pub fn get_property(&self, path: &PropertyPath) -> Option<&Property> {
        let entry = self.files.iter().find(|entry| entry.relative_path == path.relative_path)?;
        find_property(&entry.ast.items, &path.category_path, &path.property_name)
    }

    /// Sets a property's value in memory and marks its file dirty. Returns false if the path
    /// no longer resolves to an existing property (e.g. the file changed on disk since loading).
    pub fn set_property_value(&mut self, path: &PropertyPath, value: PropertyValue) -> bool {
        let Some(entry) = self.files.iter_mut().find(|entry| entry.relative_path == path.relative_path) else {
            return false;
        };
        let Some(property) = find_property_mut(&mut entry.ast.items, &path.category_path, &path.property_name)
        else {
            return false;
        };
        property.value = value;
        entry.dirty = true;
        true
    }

    /// Serializes every dirty file back to disk, clearing its dirty flag on success.
    pub fn save_dirty(&mut self) -> Vec<LoadError> {
        let mut errors = Vec::new();
        for entry in &mut self.files {
            if !entry.dirty {
                continue;
            }
            let absolute_path = self.minecraft_dir.join(&entry.relative_path);
            match std::fs::write(&absolute_path, entry.ast.to_string()) {
                Ok(()) => entry.dirty = false,
                Err(err) => {
                    errors.push(LoadError { relative_path: entry.relative_path.clone(), message: err.to_string() })
                }
            }
        }
        errors
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.files.iter().any(|entry| entry.dirty)
    }
}

fn find_property<'a>(items: &'a [Item], category_path: &[String], name: &str) -> Option<&'a Property> {
    let items = descend(items, category_path)?;
    items.iter().find_map(|item| match item {
        Item::Property(property) if property.name == name => Some(property),
        _ => None,
    })
}

fn find_property_mut<'a>(items: &'a mut [Item], category_path: &[String], name: &str) -> Option<&'a mut Property> {
    let items = descend_mut(items, category_path)?;
    items.iter_mut().find_map(|item| match item {
        Item::Property(property) if property.name == name => Some(property),
        _ => None,
    })
}

fn descend<'a>(mut items: &'a [Item], category_path: &[String]) -> Option<&'a [Item]> {
    for name in category_path {
        let category = items.iter().find_map(|item| match item {
            Item::Category(category) if &category.name == name => Some(category),
            _ => None,
        })?;
        items = &category.items;
    }
    Some(items)
}

fn descend_mut<'a>(mut items: &'a mut [Item], category_path: &[String]) -> Option<&'a mut [Item]> {
    for name in category_path {
        let category = items.iter_mut().find_map(|item| match item {
            Item::Category(category) if &category.name == name => Some(category),
            _ => None,
        })?;
        items = &mut category.items;
    }
    Some(items)
}

/// Discovers `.cfg` files across all `SCAN_ROOTS`, returning paths relative to `minecraft_dir`.
fn discover_cfg_files(minecraft_dir: &Path) -> Vec<PathBuf> {
    let mut relative_paths = Vec::new();
    for root in SCAN_ROOTS {
        relative_paths.extend(discover_cfg_files_in_root(&minecraft_dir.join(root), Path::new(root)));
    }
    relative_paths
}

/// Discovers `.cfg` files directly inside `root_dir` and up to one subdirectory deep, returning
/// paths prefixed with `relative_root` (the root's path relative to `minecraft_dir`).
fn discover_cfg_files_in_root(root_dir: &Path, relative_root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root_dir) else {
        return Vec::new();
    };

    let mut relative_paths = Vec::new();
    for entry in entries.filter_map(|entry| entry.ok()) {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name();

        if file_type.is_file() {
            if is_cfg_file(Path::new(&name)) {
                relative_paths.push(relative_root.join(&name));
            }
        } else if file_type.is_dir() {
            let Ok(sub_entries) = std::fs::read_dir(entry.path()) else {
                continue;
            };
            for sub_entry in sub_entries.filter_map(|entry| entry.ok()) {
                let sub_name = sub_entry.file_name();
                if sub_entry.path().is_file() && is_cfg_file(Path::new(&sub_name)) {
                    relative_paths.push(relative_root.join(&name).join(&sub_name));
                }
            }
        }
    }

    relative_paths
}

fn is_cfg_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("cfg"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, relative: &str, contents: &str) {
        let path = dir.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn loads_and_edits_a_property() {
        let dir = std::env::temp_dir().join(format!("gtnh-cfg-test-{}", std::process::id()));
        write(&dir, "config/modules/thing.cfg", "modules {\n    B:Flag=true\n}\n");

        let (mut store, errors) = ConfigStore::load(&dir);
        assert!(errors.is_empty());

        let path = PropertyPath {
            relative_path: PathBuf::from("config/modules/thing.cfg"),
            category_path: vec!["modules".to_string()],
            property_name: "Flag".to_string(),
        };
        assert_eq!(store.get_property(&path).unwrap().value, PropertyValue::Single("true".to_string()));

        assert!(store.set_property_value(&path, PropertyValue::Single("false".to_string())));
        assert_eq!(store.get_property(&path).unwrap().value, PropertyValue::Single("false".to_string()));
        assert!(store.has_unsaved_changes());

        assert!(store.save_dirty().is_empty());
        assert!(!store.has_unsaved_changes());

        let reloaded = std::fs::read_to_string(dir.join("config/modules/thing.cfg")).unwrap();
        assert!(reloaded.contains("B:Flag=false"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn also_discovers_serverutilities_configs() {
        let dir = std::env::temp_dir().join(format!("gtnh-cfg-test-su-{}", std::process::id()));
        write(&dir, "config/modules/thing.cfg", "modules {\n    B:Flag=true\n}\n");
        write(&dir, "serverutilities/su.cfg", "su {\n    B:Enabled=true\n}\n");
        write(&dir, "serverutilities/sub/nested.cfg", "sub {\n    B:Nested=true\n}\n");

        let (store, errors) = ConfigStore::load(&dir);
        assert!(errors.is_empty());

        let relative_paths: Vec<_> = store.files.iter().map(|entry| entry.relative_path.clone()).collect();
        assert!(relative_paths.contains(&PathBuf::from("config/modules/thing.cfg")));
        assert!(relative_paths.contains(&PathBuf::from("serverutilities/su.cfg")));
        assert!(relative_paths.contains(&PathBuf::from("serverutilities/sub/nested.cfg")));

        std::fs::remove_dir_all(&dir).ok();
    }
}
