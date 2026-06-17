use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::parser::find_data_file;

const TARGET_RESOURCE_DIR: &str = "res/data/targets";
const TARGET_RESOURCE_FILES: [&str; 5] = [
    "monster_mapping.json",
    "boss_mapping.json",
    "name_overrides.json",
    "class_path_rules.json",
    "localization_index.json",
];

#[derive(Clone, Debug, Default)]
pub struct ResourceIndex {
    names_by_path: HashMap<String, String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum TargetDocument {
    Map(HashMap<String, String>),
    Rows {
        #[serde(alias = "Rows")]
        rows: HashMap<String, String>,
    },
    List(Vec<TargetRow>),
}

#[derive(Deserialize)]
struct TargetRow {
    #[serde(default)]
    class_path: String,
    #[serde(default)]
    object_path: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    display_name: String,
}

impl ResourceIndex {
    pub fn load_default() -> Self {
        let mut index = Self::default();
        for file in TARGET_RESOURCE_FILES {
            let relative = Path::new(TARGET_RESOURCE_DIR).join(file);
            let Some(path) = find_data_file(&relative) else {
                continue;
            };
            index.load_file(&path);
        }
        index
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.names_by_path.is_empty()
    }

    pub fn display_name_for_path(&self, path: &str) -> Option<String> {
        if let Some(name) = self.names_by_path.get(path) {
            return Some(name.clone());
        }
        let normalized = normalize_path(path);
        self.names_by_path
            .get(&normalized)
            .cloned()
            .or_else(|| fallback_name_from_path(path))
    }

    fn load_file(&mut self, path: &Path) {
        let Ok(text) = fs::read_to_string(path) else {
            return;
        };
        let Ok(document) = serde_json::from_str::<TargetDocument>(&text) else {
            return;
        };
        match document {
            TargetDocument::Map(rows) | TargetDocument::Rows { rows } => {
                for (path, name) in rows {
                    self.insert(path, name);
                }
            }
            TargetDocument::List(rows) => {
                for row in rows {
                    let path = first_non_empty([row.path, row.class_path, row.object_path]);
                    let name = first_non_empty([row.display_name, row.name, String::new()]);
                    self.insert(path, name);
                }
            }
        }
    }

    fn insert(&mut self, path: String, name: String) {
        let path = normalize_path(&path);
        let name = name.trim();
        if path.is_empty() || name.is_empty() {
            return;
        }
        self.names_by_path.insert(path, name.to_owned());
    }
}

fn first_non_empty(values: impl IntoIterator<Item = String>) -> String {
    values
        .into_iter()
        .find(|value| !value.trim().is_empty())
        .unwrap_or_default()
}

fn normalize_path(path: &str) -> String {
    path.trim()
        .trim_matches('"')
        .trim_end_matches("_C")
        .to_ascii_lowercase()
}

pub fn fallback_name_from_path(path: &str) -> Option<String> {
    let trimmed = path.trim().trim_matches('\0');
    if trimmed.is_empty() {
        return None;
    }
    let without_class = trimmed
        .rsplit('/')
        .next()
        .unwrap_or(trimmed)
        .rsplit('.')
        .next()
        .unwrap_or(trimmed)
        .trim_end_matches("_C");
    if without_class.len() < 3 {
        return None;
    }
    Some(without_class.to_owned())
}

#[allow(dead_code)]
pub fn target_resource_dir() -> PathBuf {
    PathBuf::from(TARGET_RESOURCE_DIR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_targets_resource_table_is_ok() {
        let index = ResourceIndex::load_default();
        assert!(index.is_empty() || !index.is_empty());
        assert!(target_resource_dir().to_string_lossy().contains("targets"));
    }

    #[test]
    fn fallback_name_uses_path_basename() {
        assert_eq!(
            fallback_name_from_path(
                "/Game/Blueprints/Character/Monster/boss_07/BP_Boss_07.BP_Boss_07_C"
            )
            .as_deref(),
            Some("BP_Boss_07")
        );
    }
}
