//! Builds a flat, cached search index from a `ConfigStore` so filtering never has to
//! re-walk the nested AST or re-parse anything.

use std::path::Path;

use crate::config_store::{ConfigStore, PropertyPath};
use crate::forge_cfg::{Item, PropertyValue};
use crate::tree;

#[derive(Debug, Clone)]
pub struct IndexedProperty {
    pub path: PropertyPath,
    pub display_value: String,
    /// Lowercased "mod file category name value" blob used for substring matching.
    haystack: String,
}

#[derive(Debug, Default)]
pub struct SearchIndex {
    pub entries: Vec<IndexedProperty>,
}

impl SearchIndex {
    pub fn build(store: &ConfigStore) -> Self {
        let mut entries = Vec::new();
        for file in &store.files {
            collect(&file.relative_path, &[], &file.ast.items, &mut entries);
        }
        SearchIndex { entries }
    }

    /// Returns entries matching `query` (case-insensitive substring match across the file
    /// path, category path, property name and value). An empty query matches everything.
    pub fn filter<'a>(&'a self, query: &str) -> Vec<&'a IndexedProperty> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return self.entries.iter().collect();
        }
        self.entries
            .iter()
            .filter(|entry| entry.haystack.contains(&query))
            .collect()
    }

    /// Updates the cached display value (and search haystack) for a single property in place,
    /// so an edit doesn't require rebuilding the whole index.
    pub fn update_value(&mut self, path: &PropertyPath, value: &PropertyValue) {
        let Some(entry) = self.entries.iter_mut().find(|entry| &entry.path == path) else {
            return;
        };
        entry.display_value = display_value(value);
        entry.haystack = haystack(path, &entry.display_value);
    }
}

fn collect(
    relative_path: &Path,
    category_path: &[String],
    items: &[Item],
    out: &mut Vec<IndexedProperty>,
) {
    for item in items {
        match item {
            Item::Property(property) => {
                let path = PropertyPath {
                    relative_path: relative_path.to_path_buf(),
                    category_path: category_path.to_vec(),
                    property_name: property.name.clone(),
                };
                let display_value = display_value(&property.value);
                let haystack = haystack(&path, &display_value);
                out.push(IndexedProperty {
                    path,
                    display_value,
                    haystack,
                });
            }
            Item::Category(category) => {
                let mut nested = category_path.to_vec();
                nested.push(category.name.clone());
                collect(relative_path, &nested, &category.items, out);
            }
        }
    }
}

fn haystack(path: &PropertyPath, display_value: &str) -> String {
    format!(
        "{} {} {} {} {display_value}",
        tree::mod_display_name(&path.relative_path),
        path.relative_path.display(),
        path.category_path.join("/"),
        path.property_name,
    )
    .to_lowercase()
}

pub fn display_value(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Single(value) => value.clone(),
        PropertyValue::List(values) => values.join(", "),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config_store::ConfigFileEntry;
    use crate::forge_cfg;

    fn store_with(cfg: &str) -> ConfigStore {
        let ast = forge_cfg::parse(cfg).unwrap();
        ConfigStore {
            minecraft_dir: PathBuf::new(),
            files: vec![ConfigFileEntry {
                relative_path: PathBuf::from("thing.cfg"),
                ast,
                dirty: false,
            }],
        }
    }

    #[test]
    fn filters_by_name_and_value_case_insensitively() {
        let store = store_with("modules {\n    B:GenerateOil=true\n    S:Other=hello\n}\n");
        let index = SearchIndex::build(&store);
        assert_eq!(index.entries.len(), 2);

        let matches = index.filter("GENERATEOIL");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path.property_name, "GenerateOil");

        let matches = index.filter("hello");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path.property_name, "Other");

        assert_eq!(index.filter("").len(), 2);
        assert_eq!(index.filter("thing").len(), 2);
        assert_eq!(index.filter("nonexistent").len(), 0);
    }
}
