use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

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
    #[allow(dead_code)]
    pub fn load_default() -> Self {
        let mut warnings = Vec::new();
        Self::load_default_with_warnings(&mut warnings)
    }

    pub fn load_default_with_warnings(warnings: &mut Vec<String>) -> Self {
        let mut index = Self::default();
        for file in TARGET_RESOURCE_FILES {
            let relative = Path::new(TARGET_RESOURCE_DIR).join(file);
            let Some(path) = find_data_file(&relative) else {
                continue;
            };
            index.load_file_with_warnings(&path, warnings);
        }
        index
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

    pub(crate) fn load_file_with_warnings(&mut self, path: &Path, warnings: &mut Vec<String>) {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => {
                warnings.push(format!(
                    "target resource {} read failed: {error}",
                    path.display()
                ));
                return;
            }
        };
        let document = match serde_json::from_str::<TargetDocument>(&text) {
            Ok(document) => document,
            Err(error) => {
                warnings.push(format!(
                    "target resource {} has unsupported JSON structure: {error}",
                    path.display()
                ));
                return;
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn missing_targets_resource_table_is_ok() {
        let index = ResourceIndex::load_default();
        assert_eq!(
            index
                .display_name_for_path(
                    "/Game/Blueprints/Character/Monster/boss_07/BP_Boss_07.BP_Boss_07_C"
                )
                .as_deref(),
            Some("BP_Boss_07")
        );
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

    #[test]
    fn invalid_existing_target_resource_reports_warning() {
        let path = temp_resource_path("invalid-target-resource.json");
        fs::write(&path, r#"{"rows":[{"path":1}]}"#).unwrap();
        let mut warnings = Vec::new();
        let mut index = ResourceIndex::default();
        index.load_file_with_warnings(&path, &mut warnings);
        fs::remove_file(&path).ok();

        assert!(index.names_by_path.is_empty());
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("unsupported JSON structure"))
        );
    }

    #[test]
    fn valid_target_resource_loads_name() {
        let map_path = temp_resource_path("valid-target-map.json");
        let list_path = temp_resource_path("valid-target-list.json");
        let rows_path = temp_resource_path("valid-target-rows.json");
        fs::write(&map_path, r#"{"/Game/Monster/A.A_C":"Monster A"}"#).unwrap();
        fs::write(
            &list_path,
            r#"[{"path":"/Game/Monster/B.B_C","display_name":"Monster B"}]"#,
        )
        .unwrap();
        fs::write(
            &rows_path,
            r#"{"Rows":{"/Game/Monster/C.C_C":"Monster C"}}"#,
        )
        .unwrap();

        let mut warnings = Vec::new();
        let mut index = ResourceIndex::default();
        index.load_file_with_warnings(&map_path, &mut warnings);
        index.load_file_with_warnings(&list_path, &mut warnings);
        index.load_file_with_warnings(&rows_path, &mut warnings);
        fs::remove_file(&map_path).ok();
        fs::remove_file(&list_path).ok();
        fs::remove_file(&rows_path).ok();

        assert!(warnings.is_empty(), "{}", warnings.join("; "));
        assert_eq!(
            index
                .display_name_for_path("/Game/Monster/A.A_C")
                .as_deref(),
            Some("Monster A")
        );
        assert_eq!(
            index
                .display_name_for_path("/Game/Monster/B.B_C")
                .as_deref(),
            Some("Monster B")
        );
        assert_eq!(
            index
                .display_name_for_path("/Game/Monster/C.C_C")
                .as_deref(),
            Some("Monster C")
        );
    }

    fn temp_resource_path(name: &str) -> std::path::PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "nte-dps-tool-{}-{suffix}-{name}",
            std::process::id()
        ))
    }
}
