use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::io_util::atomic_write_text;

pub(crate) const MAX_MOD_SOURCE_BYTES: usize = 16 * 1024;
const MAX_MOD_BLUEPRINT_BYTES: usize = 64 * 1024;
const MAX_ENABLED_MODS: usize = 16;
const MOD_DIRECTORY_NAME: &str = "nte-mods";
const MOD_SET_FILE_NAME: &str = "nte-mods.enabled";
const MOD_BLUEPRINT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ModScriptDocument {
    pub(crate) id: String,
    pub(crate) enabled: bool,
    pub(crate) source: String,
    pub(crate) blueprint: ModScriptBlueprint,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ModScriptWorkspace {
    pub(crate) scripts: Vec<ModScriptDocument>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModScriptBlueprint {
    #[serde(default)]
    pub(crate) nodes: Vec<ModScriptBlueprintNode>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModScriptBlueprintNode {
    pub(crate) signature: String,
    pub(crate) position: [f32; 2],
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) description: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModScriptBlueprintFile {
    version: u32,
    #[serde(default)]
    nodes: Vec<ModScriptBlueprintNode>,
}

pub(crate) fn mod_script_workspace_directory() -> PathBuf {
    super::paths::software_dir().join("plugins")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModScriptError {
    FileSystem(String),
    InvalidModSet,
    DuplicateModId(String),
    InvalidModId(String),
    TooManyEnabledMods,
    SourceTooLarge,
    SourceContainsNul,
    SourceNotUtf8(String),
    BlueprintTooLarge(String),
    BlueprintNotUtf8(String),
    InvalidBlueprint(String),
    UnsupportedBlueprintVersion { id: String, version: u32 },
    MissingVersionHeader,
    MissingModDeclaration,
    MismatchedModDeclaration,
    MissingViewportTickHandler,
    ModSourceMissing(String),
}

pub(crate) fn load_mod_script_workspace(
    workspace_directory: &Path,
) -> Result<ModScriptWorkspace, ModScriptError> {
    let enabled = read_enabled_mods(workspace_directory)?;
    let mod_directory = workspace_directory.join(MOD_DIRECTORY_NAME);
    let entries = match fs::read_dir(&mod_directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ModScriptWorkspace::default());
        }
        Err(error) => return Err(ModScriptError::FileSystem(error.to_string())),
    };

    let mut scripts = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| ModScriptError::FileSystem(error.to_string()))?;
        let file_type = entry
            .file_type()
            .map_err(|error| ModScriptError::FileSystem(error.to_string()))?;
        let is_nte = entry
            .path()
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("nte"));
        if !file_type.is_file() || !is_nte {
            continue;
        }
        let id = entry
            .path()
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                ModScriptError::InvalidModId(entry.file_name().to_string_lossy().into())
            })?
            .to_owned();
        validate_mod_id(&id)?;
        let bytes = fs::read(entry.path())
            .map_err(|error| ModScriptError::FileSystem(error.to_string()))?;
        if bytes.len() > MAX_MOD_SOURCE_BYTES {
            return Err(ModScriptError::SourceTooLarge);
        }
        let source =
            String::from_utf8(bytes).map_err(|_| ModScriptError::SourceNotUtf8(id.clone()))?;
        let blueprint = load_mod_script_blueprint(&mod_directory, &id)?;
        scripts.push(ModScriptDocument {
            enabled: enabled.contains(&id),
            id,
            source,
            blueprint,
        });
    }
    scripts.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(ModScriptWorkspace { scripts })
}

pub(crate) fn save_mod_script(
    workspace_directory: &Path,
    id: &str,
    source: &str,
    blueprint: &ModScriptBlueprint,
) -> Result<(), ModScriptError> {
    validate_mod_source(id, source)?;
    let file = ModScriptBlueprintFile {
        version: MOD_BLUEPRINT_VERSION,
        nodes: blueprint.nodes.clone(),
    };
    let text = serde_json::to_string_pretty(&file)
        .map_err(|_| ModScriptError::InvalidBlueprint(id.to_owned()))?;
    if text.len() + 1 > MAX_MOD_BLUEPRINT_BYTES {
        return Err(ModScriptError::BlueprintTooLarge(id.to_owned()));
    }
    let mod_directory = workspace_directory.join(MOD_DIRECTORY_NAME);
    atomic_write_text(&mod_directory.join(format!("{id}.nte")), source)
        .map_err(ModScriptError::FileSystem)?;
    atomic_write_text(
        &mod_script_blueprint_path(&mod_directory, id),
        &format!("{text}\n"),
    )
    .map_err(ModScriptError::FileSystem)
}

fn load_mod_script_blueprint(
    mod_directory: &Path,
    id: &str,
) -> Result<ModScriptBlueprint, ModScriptError> {
    let bytes = match fs::read(mod_script_blueprint_path(mod_directory, id)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ModScriptBlueprint::default());
        }
        Err(error) => return Err(ModScriptError::FileSystem(error.to_string())),
    };
    if bytes.len() > MAX_MOD_BLUEPRINT_BYTES {
        return Err(ModScriptError::BlueprintTooLarge(id.to_owned()));
    }
    let text =
        std::str::from_utf8(&bytes).map_err(|_| ModScriptError::BlueprintNotUtf8(id.to_owned()))?;
    let file = serde_json::from_str::<ModScriptBlueprintFile>(text)
        .map_err(|_| ModScriptError::InvalidBlueprint(id.to_owned()))?;
    if file.version != MOD_BLUEPRINT_VERSION {
        return Err(ModScriptError::UnsupportedBlueprintVersion {
            id: id.to_owned(),
            version: file.version,
        });
    }
    if file.nodes.iter().any(|node| {
        node.signature.is_empty()
            || node
                .position
                .iter()
                .any(|coordinate| !coordinate.is_finite())
    }) {
        return Err(ModScriptError::InvalidBlueprint(id.to_owned()));
    }
    Ok(ModScriptBlueprint { nodes: file.nodes })
}

fn mod_script_blueprint_path(mod_directory: &Path, id: &str) -> PathBuf {
    mod_directory.join(format!("{id}.blueprint.json"))
}

pub(crate) fn set_mod_enabled(
    workspace_directory: &Path,
    id: &str,
    enabled: bool,
) -> Result<(), ModScriptError> {
    validate_mod_id(id)?;
    let mut enabled_mods = read_enabled_mods(workspace_directory)?;
    if enabled {
        if !workspace_directory
            .join(MOD_DIRECTORY_NAME)
            .join(format!("{id}.nte"))
            .is_file()
        {
            return Err(ModScriptError::ModSourceMissing(id.to_owned()));
        }
        if !enabled_mods.contains(id) {
            if enabled_mods.len() == MAX_ENABLED_MODS {
                return Err(ModScriptError::TooManyEnabledMods);
            }
            enabled_mods.insert(id.to_owned());
        }
    } else {
        enabled_mods.remove(id);
    }
    write_enabled_mods(workspace_directory, &enabled_mods)
}

pub(crate) fn validate_mod_source(id: &str, source: &str) -> Result<(), ModScriptError> {
    validate_mod_id(id)?;
    if source.len() > MAX_MOD_SOURCE_BYTES {
        return Err(ModScriptError::SourceTooLarge);
    }
    if source.contains('\0') {
        return Err(ModScriptError::SourceContainsNul);
    }
    let mut lines = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'));
    if lines.next() != Some("nte_mod(4)") {
        return Err(ModScriptError::MissingVersionHeader);
    }
    let Some(declaration) = lines.next() else {
        return Err(ModScriptError::MissingModDeclaration);
    };
    let expected_declaration = format!("mod(\"{id}\")");
    if !declaration.starts_with("mod(") {
        return Err(ModScriptError::MissingModDeclaration);
    }
    if declaration != expected_declaration {
        return Err(ModScriptError::MismatchedModDeclaration);
    }
    if !lines.any(|line| line == "def on_viewport_tick(event):") {
        return Err(ModScriptError::MissingViewportTickHandler);
    }
    Ok(())
}

pub(crate) fn new_mod_script_template(id: &str) -> Result<String, ModScriptError> {
    validate_mod_id(id)?;
    Ok(format!(
        "# Declare only the host capabilities this Mod actually uses.\n\
         nte_mod(4)\n\
         mod(\"{id}\")\n\
         requires(\"viewport.tick\")\n\
         requires(\"game.session\")\n\
         requires(\"ipc\")\n\
         route_ipc(12, \"ipc.query_mod_events\")\n\
         state.last_character = 0\n\
         \n\
         # This handler is the Mod's control flow and runs on the shared tick hook.\n\
         def on_viewport_tick(event):\n\
         \x20\x20\x20\x20character = game.player_character\n\
         \x20\x20\x20\x20if character != state.last_character:\n\
         \x20\x20\x20\x20\x20\x20\x20\x20ipc.emit(\"pre.session.changed\", state.last_character, character)\n\
         \x20\x20\x20\x20\x20\x20\x20\x20state.last_character = character\n\
         \x20\x20\x20\x20\x20\x20\x20\x20ipc.emit(\"post.session.changed\", character)\n"
    ))
}

pub(crate) fn validate_enabled_mod_set(source: &str) -> Result<(), ModScriptError> {
    parse_enabled_mods(source).map(|_| ())
}

fn validate_mod_id(id: &str) -> Result<(), ModScriptError> {
    if id.is_empty()
        || id.len() > 31
        || id.bytes().any(|value| {
            !value.is_ascii_lowercase()
                && !value.is_ascii_digit()
                && !matches!(value, b'-' | b'_' | b'.')
        })
    {
        return Err(ModScriptError::InvalidModId(id.to_owned()));
    }
    Ok(())
}

fn read_enabled_mods(workspace_directory: &Path) -> Result<HashSet<String>, ModScriptError> {
    let path = workspace_directory.join(MOD_SET_FILE_NAME);
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(HashSet::new());
        }
        Err(error) => return Err(ModScriptError::FileSystem(error.to_string())),
    };
    parse_enabled_mods(&text)
}

fn parse_enabled_mods(text: &str) -> Result<HashSet<String>, ModScriptError> {
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'));
    if lines.next() != Some("nte_mod_set 1") {
        return Err(ModScriptError::InvalidModSet);
    }
    let mut enabled = HashSet::new();
    for line in lines {
        let Some(id) = line.strip_prefix("load ") else {
            return Err(ModScriptError::InvalidModSet);
        };
        validate_mod_id(id)?;
        if !enabled.insert(id.to_owned()) {
            return Err(ModScriptError::DuplicateModId(id.to_owned()));
        }
    }
    if enabled.len() > MAX_ENABLED_MODS {
        return Err(ModScriptError::TooManyEnabledMods);
    }
    Ok(enabled)
}

fn write_enabled_mods(
    workspace_directory: &Path,
    enabled: &HashSet<String>,
) -> Result<(), ModScriptError> {
    let mut ids: Vec<_> = enabled.iter().map(String::as_str).collect();
    ids.sort_unstable();
    let mut text = String::from("nte_mod_set 1\n");
    for id in ids {
        text.push_str("load ");
        text.push_str(id);
        text.push('\n');
    }
    atomic_write_text(&workspace_directory.join(MOD_SET_FILE_NAME), &text)
        .map_err(ModScriptError::FileSystem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_workspace() -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "nte-mod-script-test-{}-{unique}",
            std::process::id()
        ))
    }

    #[test]
    fn workspace_round_trip_preserves_source_and_enabled_state() {
        let root = temp_workspace();
        let source = new_mod_script_template("telemetry").unwrap();
        assert!(source.starts_with("# Declare only the host capabilities"));
        assert!(source.contains("# This handler is the Mod's control flow"));
        assert!(source.contains("game.player_character"));
        assert!(source.contains("ipc.emit(\"pre.session.changed\""));
        assert!(source.contains("ipc.emit(\"post.session.changed\""));
        let blueprint = ModScriptBlueprint {
            nodes: vec![ModScriptBlueprintNode {
                signature: "0:character = game.player_character\n".to_owned(),
                position: [184.5, 92.25],
                description: "Read the active character.".to_owned(),
            }],
        };

        save_mod_script(&root, "telemetry", &source, &blueprint).unwrap();
        set_mod_enabled(&root, "telemetry", true).unwrap();
        let workspace = load_mod_script_workspace(&root).unwrap();

        assert_eq!(
            workspace.scripts,
            vec![ModScriptDocument {
                id: "telemetry".to_owned(),
                enabled: true,
                source,
                blueprint,
            }]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workspace_loads_legacy_script_without_blueprint_metadata() {
        let root = temp_workspace();
        let source = new_mod_script_template("legacy").unwrap();
        atomic_write_text(&root.join("nte-mods").join("legacy.nte"), &source).unwrap();

        let workspace = load_mod_script_workspace(&root).unwrap();

        assert_eq!(
            workspace.scripts[0].blueprint,
            ModScriptBlueprint::default()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn oversized_blueprint_is_rejected_before_source_is_written() {
        let root = temp_workspace();
        let source = new_mod_script_template("oversized").unwrap();
        let blueprint = ModScriptBlueprint {
            nodes: vec![ModScriptBlueprintNode {
                signature: "0:value = 1\n".to_owned(),
                position: [0.0, 0.0],
                description: "x".repeat(MAX_MOD_BLUEPRINT_BYTES),
            }],
        };

        assert_eq!(
            save_mod_script(&root, "oversized", &source, &blueprint),
            Err(ModScriptError::BlueprintTooLarge("oversized".to_owned()))
        );
        assert!(!root.join("nte-mods").join("oversized.nte").exists());
    }

    #[test]
    fn source_validation_requires_matching_v4_declaration_and_handler() {
        assert_eq!(
            validate_mod_source(
                "telemetry",
                "nte_mod(4)\nmod(\"other\")\ndef on_viewport_tick(event):\n    value = 1\n"
            ),
            Err(ModScriptError::MismatchedModDeclaration)
        );
        assert_eq!(
            validate_mod_source("telemetry", "nte_mod(4)\nmod(\"telemetry\")\n"),
            Err(ModScriptError::MissingViewportTickHandler)
        );
    }

    #[test]
    fn enabled_set_rejects_duplicates_and_path_characters() {
        assert_eq!(
            parse_enabled_mods("nte_mod_set 1\nload telemetry\nload telemetry\n"),
            Err(ModScriptError::DuplicateModId("telemetry".to_owned()))
        );
        assert_eq!(
            parse_enabled_mods("nte_mod_set 1\nload ../telemetry\n"),
            Err(ModScriptError::InvalidModId("../telemetry".to_owned()))
        );
    }
}
