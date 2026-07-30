//! Frontend-neutral read model for the software-side Mod workspace.
//!
//! The workspace is available independently from game installation detection.
//! Mutating operations remain in the existing storage/deployment transaction
//! until a later Mod Studio slice adds explicit commands for them.

use std::path::{Path, PathBuf};

use crate::storage::mod_scripts::{
    ModScriptDocument, ModScriptError, ModScriptWorkspace, load_mod_script_workspace,
    mod_script_workspace_directory, validate_mod_id,
};

pub const MOD_STUDIO_WORKSPACE_LABEL: &str = "plugins/nte-mods";
pub const MAX_MOD_STUDIO_DOCUMENTS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModStudioDocumentSummary {
    pub id: String,
    pub enabled: bool,
    pub source_bytes: u32,
    pub line_count: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModStudioWorkspace {
    pub documents: Vec<ModStudioDocumentSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModStudioDocument {
    pub id: String,
    pub enabled: bool,
    pub source: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModStudioErrorCode {
    FileSystem,
    InvalidWorkspace,
    InvalidModId,
    DocumentNotFound,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModStudioError {
    pub code: ModStudioErrorCode,
    pub detail: String,
}

impl ModStudioError {
    fn new(code: ModStudioErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

pub fn default_mod_studio_workspace_directory() -> PathBuf {
    let portable = mod_script_workspace_directory();
    #[cfg(debug_assertions)]
    {
        let development = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins");
        if !portable.join("nte-mods").is_dir() && development.join("nte-mods").is_dir() {
            return development;
        }
    }
    portable
}

pub fn load_default_mod_studio_workspace() -> Result<ModStudioWorkspace, ModStudioError> {
    load_mod_studio_workspace(&default_mod_studio_workspace_directory())
}

pub fn load_default_mod_studio_document(id: &str) -> Result<ModStudioDocument, ModStudioError> {
    load_mod_studio_document(&default_mod_studio_workspace_directory(), id)
}

pub fn load_mod_studio_workspace(
    workspace_directory: &Path,
) -> Result<ModStudioWorkspace, ModStudioError> {
    let workspace = load_bounded_workspace(workspace_directory)?;
    Ok(ModStudioWorkspace {
        documents: workspace
            .scripts
            .iter()
            .map(project_document_summary)
            .collect(),
    })
}

pub fn load_mod_studio_document(
    workspace_directory: &Path,
    id: &str,
) -> Result<ModStudioDocument, ModStudioError> {
    validate_mod_id(id).map_err(|error| {
        ModStudioError::new(
            ModStudioErrorCode::InvalidModId,
            storage_error_detail(error),
        )
    })?;
    let workspace = load_bounded_workspace(workspace_directory)?;
    let document = workspace
        .scripts
        .into_iter()
        .find(|document| document.id == id)
        .ok_or_else(|| {
            ModStudioError::new(
                ModStudioErrorCode::DocumentNotFound,
                format!("Mod document {id:?} was not present in the workspace"),
            )
        })?;
    Ok(ModStudioDocument {
        id: document.id,
        enabled: document.enabled,
        source: document.source,
    })
}

fn load_bounded_workspace(
    workspace_directory: &Path,
) -> Result<ModScriptWorkspace, ModStudioError> {
    let workspace = load_mod_script_workspace(workspace_directory).map_err(map_storage_error)?;
    if workspace.scripts.len() > MAX_MOD_STUDIO_DOCUMENTS {
        return Err(ModStudioError::new(
            ModStudioErrorCode::InvalidWorkspace,
            format!(
                "Mod workspace contains {} documents; maximum is {MAX_MOD_STUDIO_DOCUMENTS}",
                workspace.scripts.len()
            ),
        ));
    }
    Ok(workspace)
}

fn project_document_summary(document: &ModScriptDocument) -> ModStudioDocumentSummary {
    ModStudioDocumentSummary {
        id: document.id.clone(),
        enabled: document.enabled,
        source_bytes: document.source.len() as u32,
        line_count: document.source.lines().count() as u32,
    }
}

fn map_storage_error(error: ModScriptError) -> ModStudioError {
    let code = match error {
        ModScriptError::FileSystem(_) => ModStudioErrorCode::FileSystem,
        ModScriptError::InvalidModId(_) => ModStudioErrorCode::InvalidWorkspace,
        ModScriptError::InvalidModSet
        | ModScriptError::DuplicateModId(_)
        | ModScriptError::TooManyEnabledMods
        | ModScriptError::SourceTooLarge
        | ModScriptError::SourceContainsNul
        | ModScriptError::SourceNotUtf8(_)
        | ModScriptError::MissingVersionHeader
        | ModScriptError::MissingModDeclaration
        | ModScriptError::MismatchedModDeclaration
        | ModScriptError::MissingViewportTickHandler
        | ModScriptError::InvalidSourceLine(_)
        | ModScriptError::SourceBudgetExceeded
        | ModScriptError::CapabilityMismatch
        | ModScriptError::ModSourceMissing(_) => ModStudioErrorCode::InvalidWorkspace,
    };
    ModStudioError::new(code, storage_error_detail(error))
}

fn storage_error_detail(error: ModScriptError) -> String {
    match error {
        ModScriptError::FileSystem(detail) => detail,
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::storage::mod_scripts::{new_mod_script_template, save_mod_script, set_mod_enabled};

    use super::*;

    fn temp_workspace(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "nte-mod-studio-{label}-{}-{unique}",
            std::process::id()
        ))
    }

    #[test]
    fn empty_workspace_is_a_valid_read_model() {
        let root = temp_workspace("empty");

        assert_eq!(
            load_mod_studio_workspace(&root).expect("empty workspace"),
            ModStudioWorkspace::default()
        );
    }

    #[test]
    fn workspace_summary_and_document_share_the_storage_fact_source() {
        let root = temp_workspace("round-trip");
        let source = new_mod_script_template("telemetry").expect("template");
        save_mod_script(&root, "telemetry", &source).expect("save");
        set_mod_enabled(&root, "telemetry", true).expect("enable");

        let workspace = load_mod_studio_workspace(&root).expect("workspace");
        assert_eq!(
            workspace.documents,
            vec![ModStudioDocumentSummary {
                id: "telemetry".to_owned(),
                enabled: true,
                source_bytes: source.len() as u32,
                line_count: source.lines().count() as u32,
            }]
        );
        assert_eq!(
            load_mod_studio_document(&root, "telemetry").expect("document"),
            ModStudioDocument {
                id: "telemetry".to_owned(),
                enabled: true,
                source,
            }
        );

        fs::remove_dir_all(root).expect("remove workspace");
    }

    #[test]
    fn document_id_is_validated_before_workspace_lookup() {
        let root = temp_workspace("invalid-id");
        let error = load_mod_studio_document(&root, "../telemetry").expect_err("invalid id");

        assert_eq!(error.code, ModStudioErrorCode::InvalidModId);
        assert!(!root.exists());
    }

    #[test]
    fn missing_document_has_a_stable_error_category() {
        let root = temp_workspace("missing");
        let error = load_mod_studio_document(&root, "telemetry").expect_err("missing document");

        assert_eq!(error.code, ModStudioErrorCode::DocumentNotFound);
    }

    #[test]
    fn default_workspace_label_does_not_expose_an_absolute_user_path() {
        assert_eq!(MOD_STUDIO_WORKSPACE_LABEL, "plugins/nte-mods");
        assert!(default_mod_studio_workspace_directory().ends_with("plugins"));
    }

    #[test]
    fn oversized_workspace_is_rejected_before_projection() {
        let root = temp_workspace("oversized");
        let mod_directory = root.join("nte-mods");
        fs::create_dir_all(&mod_directory).expect("create workspace");
        for index in 0..=MAX_MOD_STUDIO_DOCUMENTS {
            fs::write(
                mod_directory.join(format!("mod-{index:03}.nte")),
                "NTE_SCRIPT(5);",
            )
            .expect("write document");
        }

        let error = load_mod_studio_workspace(&root).expect_err("oversized workspace");
        assert_eq!(error.code, ModStudioErrorCode::InvalidWorkspace);

        fs::remove_dir_all(root).expect("remove workspace");
    }
}
