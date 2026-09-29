//! Navigation model for the config browser: groups loaded files into "mods" and exposes each
//! file's category hierarchy, so the GUI only has to render it.

use std::path::{Path, PathBuf};

use crate::config_store::{ConfigStore, PropertyPath};
use crate::forge_cfg::{ConfigFile, Item};

/// Separator used when rendering breadcrumbs.
pub const CRUMB_SEPARATOR: &str = " › ";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    Config,
    ServerUtilities,
}

impl Section {
    pub fn label(self) -> &'static str {
        match self {
            Section::Config => "Mods",
            Section::ServerUtilities => "ServerUtilities",
        }
    }
}

/// A mod as shown in the sidebar: either a subfolder of a scan root (all its files), or a
/// single loose file directly inside a scan root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModGroup {
    /// Stable key: the subfolder or loose file path, relative to `.minecraft`.
    pub id: String,
    pub display_name: String,
    pub section: Section,
    /// Files in this group, relative to `.minecraft`, sorted.
    pub files: Vec<PathBuf>,
}

/// A category and its nested subcategories within one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryNode {
    pub name: String,
    /// Full category path from the file's top level, including `name`.
    pub path: Vec<String>,
    pub children: Vec<CategoryNode>,
    /// Number of properties directly inside this category (not counting subcategories).
    pub property_count: usize,
}

/// Groups every loaded file into mods, sorted by section and then case-insensitively by name.
pub fn group_mods(store: &ConfigStore) -> Vec<ModGroup> {
    let mut groups: Vec<ModGroup> = Vec::new();
    for file in &store.files {
        let id = mod_id_for(&file.relative_path);
        match groups.iter_mut().find(|group| group.id == id) {
            Some(group) => group.files.push(file.relative_path.clone()),
            None => groups.push(ModGroup {
                display_name: mod_display_name(&file.relative_path),
                section: section_for(&file.relative_path),
                files: vec![file.relative_path.clone()],
                id,
            }),
        }
    }
    for group in &mut groups {
        group.files.sort();
    }
    groups.sort_by(|a, b| {
        a.section.cmp(&b.section).then_with(|| {
            a.display_name
                .to_lowercase()
                .cmp(&b.display_name.to_lowercase())
        })
    });
    groups
}

/// The id of the mod group `relative_path` belongs to: `root/<dir>` for files in a subfolder,
/// otherwise the file's own path.
pub fn mod_id_for(relative_path: &Path) -> String {
    let components: Vec<_> = relative_path.components().collect();
    let key: PathBuf = if components.len() > 2 {
        components[..2].iter().collect()
    } else {
        relative_path.to_path_buf()
    };
    key.to_string_lossy().replace('\\', "/")
}

/// The display name of the mod group `relative_path` belongs to: the subfolder name, or the
/// loose file's stem.
pub fn mod_display_name(relative_path: &Path) -> String {
    let components: Vec<_> = relative_path.components().collect();
    if components.len() > 2 {
        components[1].as_os_str().to_string_lossy().into_owned()
    } else {
        file_stem(relative_path)
    }
}

pub fn file_name(relative_path: &Path) -> String {
    relative_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn file_stem(relative_path: &Path) -> String {
    relative_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn section_for(relative_path: &Path) -> Section {
    match relative_path.components().next() {
        Some(root) if root.as_os_str() == "serverutilities" => Section::ServerUtilities,
        _ => Section::Config,
    }
}

/// The category hierarchy of a parsed file.
pub fn category_tree(file: &ConfigFile) -> Vec<CategoryNode> {
    collect_categories(&file.items, &[])
}

fn collect_categories(items: &[Item], parent: &[String]) -> Vec<CategoryNode> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Category(category) => {
                let mut path = parent.to_vec();
                path.push(category.name.clone());
                Some(CategoryNode {
                    name: category.name.clone(),
                    children: collect_categories(&category.items, &path),
                    property_count: category
                        .items
                        .iter()
                        .filter(|item| matches!(item, Item::Property(_)))
                        .count(),
                    path,
                })
            }
            Item::Property(_) => None,
        })
        .collect()
}

/// "Mod › file.cfg › category › subcategory" for the location containing `path`. The file
/// name is left out when it just repeats the mod name.
pub fn breadcrumb(path: &PropertyPath) -> String {
    location_breadcrumb(&path.relative_path, &path.category_path)
}

pub fn location_breadcrumb(relative_path: &Path, category_path: &[String]) -> String {
    let mod_name = mod_display_name(relative_path);
    let mut crumbs = vec![mod_name.clone()];
    if file_stem(relative_path) != mod_name {
        crumbs.push(file_name(relative_path));
    }
    crumbs.extend(category_path.iter().cloned());
    crumbs.join(CRUMB_SEPARATOR)
}

/// The property's index among the properties directly in its category, and how many there are.
pub fn property_position(store: &ConfigStore, path: &PropertyPath) -> Option<(usize, usize)> {
    let items = store.items_at(&path.relative_path, &path.category_path)?;
    let names: Vec<&str> = items
        .iter()
        .filter_map(|item| match item {
            Item::Property(property) => Some(property.name.as_str()),
            Item::Category(_) => None,
        })
        .collect();
    let index = names.iter().position(|name| *name == path.property_name)?;
    Some((index, names.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_store::ConfigFileEntry;
    use crate::forge_cfg;

    fn store_with(files: &[(&str, &str)]) -> ConfigStore {
        ConfigStore {
            minecraft_dir: PathBuf::new(),
            files: files
                .iter()
                .map(|(path, cfg)| ConfigFileEntry {
                    relative_path: PathBuf::from(path),
                    ast: forge_cfg::parse(cfg).unwrap(),
                    dirty: false,
                })
                .collect(),
        }
    }

    #[test]
    fn groups_subfolders_loose_files_and_serverutilities() {
        let store = store_with(&[
            ("config/GTNewHorizons/dreamcraft.cfg", "a {\n}\n"),
            ("config/GTNewHorizons/other.cfg", "a {\n}\n"),
            ("config/ironchest.cfg", "a {\n}\n"),
            ("serverutilities/serverutilities.cfg", "a {\n}\n"),
            ("config/Aroma.cfg", "a {\n}\n"),
        ]);
        let groups = group_mods(&store);
        let names: Vec<_> = groups.iter().map(|g| g.display_name.as_str()).collect();
        assert_eq!(
            names,
            ["Aroma", "GTNewHorizons", "ironchest", "serverutilities"]
        );

        assert_eq!(groups[1].id, "config/GTNewHorizons");
        assert_eq!(groups[1].files.len(), 2);
        assert_eq!(groups[2].id, "config/ironchest.cfg");
        assert_eq!(groups[3].section, Section::ServerUtilities);

        for group in &groups {
            for file in &group.files {
                assert_eq!(mod_id_for(file), group.id);
            }
        }
    }

    #[test]
    fn builds_nested_category_tree() {
        let store = store_with(&[(
            "config/thing.cfg",
            "outer {\n    B:A=true\n    inner {\n        I:B=1\n        I:C=2\n    }\n}\nsecond {\n}\n",
        )]);
        let tree = category_tree(&store.files[0].ast);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].name, "outer");
        assert_eq!(tree[0].property_count, 1);
        assert_eq!(tree[0].children[0].path, ["outer", "inner"]);
        assert_eq!(tree[0].children[0].property_count, 2);
        assert!(tree[1].children.is_empty());
    }

    #[test]
    fn breadcrumbs_and_positions() {
        let store = store_with(&[(
            "config/GTNewHorizons/dreamcraft.cfg",
            "modules {\n    B:A=true\n    B:B=true\n    sub {\n        I:C=1\n    }\n    B:D=true\n}\n",
        )]);
        let path = PropertyPath {
            relative_path: PathBuf::from("config/GTNewHorizons/dreamcraft.cfg"),
            category_path: vec!["modules".to_string()],
            property_name: "D".to_string(),
        };
        assert_eq!(
            breadcrumb(&path),
            "GTNewHorizons › dreamcraft.cfg › modules"
        );
        assert_eq!(property_position(&store, &path), Some((2, 3)));
        assert_eq!(
            location_breadcrumb(Path::new("config/ironchest.cfg"), &[]),
            "ironchest"
        );
    }
}
