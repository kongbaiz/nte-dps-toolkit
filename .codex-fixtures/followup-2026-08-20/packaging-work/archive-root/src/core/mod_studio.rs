//! Frontend-neutral read model for the software-side Mod workspace.
//!
//! The workspace is available independently from game installation detection.
//! Source saves and enabled-set changes reuse the existing validation and
//! atomic storage transactions; deployment remains a separate operation.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::storage::mod_scripts::{
    ModScriptDocument, ModScriptError, ModScriptWorkspace, delete_mod_script, is_mod_binding_id,
    load_mod_script_workspace, mod_script_workspace_directory, mod_source_bindings,
    new_mod_script_template, save_mod_script, set_mod_enabled, validate_mod_id,
};

pub const MOD_STUDIO_WORKSPACE_LABEL: &str = "plugins/nte-mods";
pub const MAX_MOD_STUDIO_DOCUMENTS: usize = 256;
pub const MAX_MOD_STUDIO_RUNTIME_LOGS: usize = 18;
pub const MAX_MOD_STUDIO_RUNTIME_EVENTS: usize = 18;
pub const MOD_BINDING_DPS_TIME_STOP: &str = "feature.dps-time-stop";
pub const MOD_BINDING_EMPTY_CURTAIN_EQUIPMENT: &str = "feature.empty-curtain-equipment";

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
pub struct VersionedModStudioWorkspace {
    pub generation: u64,
    pub workspace: ModStudioWorkspace,
}

#[derive(Clone)]
pub struct ModStudioWorkspaceService(Arc<ModStudioWorkspaceServiceInner>);

struct ModStudioWorkspaceServiceInner {
    workspace_directory: PathBuf,
    transaction: Mutex<()>,
    generation: AtomicU64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModStudioDocument {
    pub id: String,
    pub enabled: bool,
    pub source: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModStudioRuntimeLevel {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModStudioRuntimeLog {
    pub sequence: u64,
    pub timestamp_100ns: u64,
    pub mod_id: String,
    pub level: ModStudioRuntimeLevel,
    pub message: String,
    pub message_key: Option<&'static str>,
    pub message_arguments: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModStudioRuntimeEvent {
    pub sequence: u64,
    pub timestamp_100ns: u64,
    pub mod_id: String,
    pub name: String,
    pub values: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModStudioRuntimeSnapshot {
    pub logs: Vec<ModStudioRuntimeLog>,
    pub events: Vec<ModStudioRuntimeEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModStudioErrorCode {
    FileSystem,
    WriteFailed,
    EnabledSetWriteFailed,
    DeleteFailed,
    InvalidWorkspace,
    InvalidModId,
    DocumentAlreadyExists,
    DocumentNotFound,
    SourceTooLarge,
    SourceContainsNul,
    MissingVersionHeader,
    MissingModDeclaration,
    MismatchedModDeclaration,
    MissingViewportTickHandler,
    InvalidSourceLine,
    SourceBudgetExceeded,
    CapabilityMismatch,
    TooManyEnabledMods,
    ModSourceMissing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModStudioError {
    pub code: ModStudioErrorCode,
    pub detail: String,
    pub diagnostic_line: Option<u32>,
}

impl ModStudioError {
    fn new(code: ModStudioErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
            diagnostic_line: None,
        }
    }

    fn at_line(mut self, line: usize) -> Self {
        self.diagnostic_line = Some(line as u32);
        self
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

impl Default for ModStudioWorkspaceService {
    fn default() -> Self {
        Self::new(default_mod_studio_workspace_directory())
    }
}

impl ModStudioWorkspaceService {
    pub fn new(workspace_directory: PathBuf) -> Self {
        Self(Arc::new(ModStudioWorkspaceServiceInner {
            workspace_directory,
            transaction: Mutex::new(()),
            generation: AtomicU64::new(0),
        }))
    }

    pub fn load_workspace(&self) -> Result<VersionedModStudioWorkspace, ModStudioError> {
        let _transaction = self.lock_transaction()?;
        self.versioned_workspace()
    }

    pub fn load_document(&self, id: &str) -> Result<ModStudioDocument, ModStudioError> {
        let _transaction = self.lock_transaction()?;
        load_mod_studio_document(&self.0.workspace_directory, id)
    }

    pub fn create_document(&self, id: &str) -> Result<ModStudioDocument, ModStudioError> {
        let _transaction = self.lock_transaction()?;
        let document = create_mod_studio_document(&self.0.workspace_directory, id)?;
        self.bump_generation();
        Ok(document)
    }

    pub fn workspace_directory(&self) -> PathBuf {
        self.0.workspace_directory.clone()
    }

    pub fn enabled_binding_provider(
        &self,
        binding: &str,
    ) -> Result<Option<String>, ModStudioError> {
        let _transaction = self.lock_transaction()?;
        if !is_mod_binding_id(binding) {
            return Err(ModStudioError::new(
                ModStudioErrorCode::InvalidWorkspace,
                "application Mod binding ID is invalid",
            ));
        }
        let workspace = load_bounded_workspace(&self.0.workspace_directory)?;
        for document in workspace
            .scripts
            .into_iter()
            .filter(|document| document.enabled)
        {
            let bindings =
                mod_source_bindings(&document.id, &document.source).map_err(map_storage_error)?;
            if bindings.iter().any(|declared| declared == binding) {
                return Ok(Some(document.id));
            }
        }
        Ok(None)
    }

    pub fn save_document(
        &self,
        id: &str,
        source: &str,
    ) -> Result<ModStudioDocument, ModStudioError> {
        let _transaction = self.lock_transaction()?;
        let document = save_mod_studio_document(&self.0.workspace_directory, id, source)?;
        self.bump_generation();
        Ok(document)
    }

    pub fn install_market_document(
        &self,
        id: &str,
        source: &str,
    ) -> Result<ModStudioDocument, ModStudioError> {
        let _transaction = self.lock_transaction()?;
        let workspace = load_bounded_workspace(&self.0.workspace_directory)?;
        if workspace.scripts.iter().any(|document| document.id == id) {
            return Err(ModStudioError::new(
                ModStudioErrorCode::DocumentAlreadyExists,
                format!("Mod document {id:?} already exists"),
            ));
        }
        save_mod_script(&self.0.workspace_directory, id, source).map_err(map_save_error)?;
        if let Err(error) = set_mod_enabled(&self.0.workspace_directory, id, true) {
            let rollback = delete_mod_script(&self.0.workspace_directory, id);
            let mut mapped = map_enabled_set_error(error);
            if let Err(rollback_error) = rollback {
                mapped.detail = format!(
                    "{}; market install rollback failed: {}",
                    mapped.detail,
                    storage_error_detail(rollback_error)
                );
            }
            return Err(mapped);
        }
        self.bump_generation();
        Ok(ModStudioDocument {
            id: id.to_owned(),
            enabled: true,
            source: source.to_owned(),
        })
    }

    pub fn set_document_enabled(
        &self,
        id: &str,
        enabled: bool,
    ) -> Result<VersionedModStudioWorkspace, ModStudioError> {
        let _transaction = self.lock_transaction()?;
        let workspace = set_mod_studio_document_enabled(&self.0.workspace_directory, id, enabled)?;
        let generation = self.bump_generation();
        Ok(VersionedModStudioWorkspace {
            generation,
            workspace,
        })
    }

    pub fn delete_document(&self, id: &str) -> Result<VersionedModStudioWorkspace, ModStudioError> {
        let _transaction = self.lock_transaction()?;
        load_mod_studio_document(&self.0.workspace_directory, id)?;
        delete_mod_script(&self.0.workspace_directory, id).map_err(map_delete_error)?;
        let workspace = load_mod_studio_workspace(&self.0.workspace_directory)?;
        let generation = self.bump_generation();
        Ok(VersionedModStudioWorkspace {
            generation,
            workspace,
        })
    }

    fn lock_transaction(&self) -> Result<std::sync::MutexGuard<'_, ()>, ModStudioError> {
        self.0.transaction.lock().map_err(|_| {
            ModStudioError::new(
                ModStudioErrorCode::FileSystem,
                "Mod workspace transaction state is unavailable",
            )
        })
    }

    fn versioned_workspace(&self) -> Result<VersionedModStudioWorkspace, ModStudioError> {
        Ok(VersionedModStudioWorkspace {
            generation: self.0.generation.load(Ordering::Acquire),
            workspace: load_mod_studio_workspace(&self.0.workspace_directory)?,
        })
    }

    fn bump_generation(&self) -> u64 {
        let generation = self.0.generation.load(Ordering::Acquire).saturating_add(1);
        self.0.generation.store(generation, Ordering::Release);
        generation
    }
}

pub fn load_default_mod_studio_workspace() -> Result<ModStudioWorkspace, ModStudioError> {
    load_mod_studio_workspace(&default_mod_studio_workspace_directory())
}

pub fn load_default_mod_studio_document(id: &str) -> Result<ModStudioDocument, ModStudioError> {
    load_mod_studio_document(&default_mod_studio_workspace_directory(), id)
}

pub fn save_default_mod_studio_document(
    id: &str,
    source: &str,
) -> Result<ModStudioDocument, ModStudioError> {
    save_mod_studio_document(&default_mod_studio_workspace_directory(), id, source)
}

pub fn set_default_mod_studio_document_enabled(
    id: &str,
    enabled: bool,
) -> Result<ModStudioWorkspace, ModStudioError> {
    set_mod_studio_document_enabled(&default_mod_studio_workspace_directory(), id, enabled)
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

pub fn save_mod_studio_document(
    workspace_directory: &Path,
    id: &str,
    source: &str,
) -> Result<ModStudioDocument, ModStudioError> {
    let existing = load_mod_studio_document(workspace_directory, id)?;
    save_mod_script(workspace_directory, id, source).map_err(map_save_error)?;
    Ok(ModStudioDocument {
        id: existing.id,
        enabled: existing.enabled,
        source: source.to_owned(),
    })
}

pub fn create_mod_studio_document(
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
    if workspace.scripts.iter().any(|document| document.id == id) {
        return Err(ModStudioError::new(
            ModStudioErrorCode::DocumentAlreadyExists,
            format!("Mod document {id:?} already exists"),
        ));
    }
    let source = new_mod_script_template(id).map_err(map_save_error)?;
    save_mod_script(workspace_directory, id, &source).map_err(map_save_error)?;
    Ok(ModStudioDocument {
        id: id.to_owned(),
        enabled: false,
        source,
    })
}

pub fn set_mod_studio_document_enabled(
    workspace_directory: &Path,
    id: &str,
    enabled: bool,
) -> Result<ModStudioWorkspace, ModStudioError> {
    load_mod_studio_document(workspace_directory, id)?;
    set_mod_enabled(workspace_directory, id, enabled).map_err(map_enabled_set_error)?;
    load_mod_studio_workspace(workspace_directory)
}

#[cfg(feature = "desktop")]
pub fn poll_mod_studio_runtime() -> Result<ModStudioRuntimeSnapshot, ModStudioError> {
    use crate::platform::mods_plugin::{query_mod_events, query_mod_logs};

    let mut logs = query_mod_logs()
        .map_err(|detail| ModStudioError::new(ModStudioErrorCode::FileSystem, detail))?
        .into_iter()
        .map(project_runtime_log)
        .collect::<Vec<_>>();
    logs.sort_by_key(|entry| entry.sequence);
    if logs.len() > MAX_MOD_STUDIO_RUNTIME_LOGS {
        logs.drain(..logs.len() - MAX_MOD_STUDIO_RUNTIME_LOGS);
    }
    let mut events = query_mod_events()
        .map_err(|detail| ModStudioError::new(ModStudioErrorCode::FileSystem, detail))?
        .into_iter()
        .map(project_runtime_event)
        .collect::<Vec<_>>();
    events.sort_by_key(|entry| entry.sequence);
    if events.len() > MAX_MOD_STUDIO_RUNTIME_EVENTS {
        events.drain(..events.len() - MAX_MOD_STUDIO_RUNTIME_EVENTS);
    }
    Ok(ModStudioRuntimeSnapshot { logs, events })
}

#[cfg(feature = "desktop")]
fn project_runtime_log(entry: crate::platform::mods_plugin::ModLogSnapshot) -> ModStudioRuntimeLog {
    use crate::platform::mods_plugin::ModLogLevel;

    let level = match entry.level {
        ModLogLevel::Info => ModStudioRuntimeLevel::Info,
        ModLogLevel::Warning => ModStudioRuntimeLevel::Warning,
        ModLogLevel::Error => ModStudioRuntimeLevel::Error,
    };
    let message_key = runtime_message_key(&entry.message);
    ModStudioRuntimeLog {
        sequence: entry.sequence,
        timestamp_100ns: entry.timestamp_100ns,
        mod_id: entry.mod_id,
        level,
        message: entry.message,
        message_key,
        message_arguments: Vec::new(),
    }
}

#[cfg(feature = "desktop")]
fn project_runtime_event(
    entry: crate::platform::mods_plugin::ModEventSnapshot,
) -> ModStudioRuntimeEvent {
    ModStudioRuntimeEvent {
        sequence: entry.sequence,
        timestamp_100ns: entry.timestamp_100ns,
        mod_id: entry.mod_id,
        name: entry.name,
        values: entry.values,
    }
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
        | ModScriptError::TooManyScripts
        | ModScriptError::TooManyWorkspaceEntries
        | ModScriptError::WorkspaceTooLarge
        | ModScriptError::ModSetTooLarge
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

fn map_save_error(error: ModScriptError) -> ModStudioError {
    let detail = storage_error_detail(error.clone());
    match error {
        ModScriptError::FileSystem(_) => {
            ModStudioError::new(ModStudioErrorCode::WriteFailed, detail)
        }
        ModScriptError::InvalidModId(_) => {
            ModStudioError::new(ModStudioErrorCode::InvalidModId, detail)
        }
        ModScriptError::SourceTooLarge => {
            ModStudioError::new(ModStudioErrorCode::SourceTooLarge, detail)
        }
        ModScriptError::SourceContainsNul => {
            ModStudioError::new(ModStudioErrorCode::SourceContainsNul, detail)
        }
        ModScriptError::MissingVersionHeader => {
            ModStudioError::new(ModStudioErrorCode::MissingVersionHeader, detail)
        }
        ModScriptError::MissingModDeclaration => {
            ModStudioError::new(ModStudioErrorCode::MissingModDeclaration, detail)
        }
        ModScriptError::MismatchedModDeclaration => {
            ModStudioError::new(ModStudioErrorCode::MismatchedModDeclaration, detail)
        }
        ModScriptError::MissingViewportTickHandler => {
            ModStudioError::new(ModStudioErrorCode::MissingViewportTickHandler, detail)
        }
        ModScriptError::InvalidSourceLine(line) => {
            ModStudioError::new(ModStudioErrorCode::InvalidSourceLine, detail).at_line(line)
        }
        ModScriptError::SourceBudgetExceeded => {
            ModStudioError::new(ModStudioErrorCode::SourceBudgetExceeded, detail)
        }
        ModScriptError::CapabilityMismatch => {
            ModStudioError::new(ModStudioErrorCode::CapabilityMismatch, detail)
        }
        ModScriptError::InvalidModSet
        | ModScriptError::DuplicateModId(_)
        | ModScriptError::TooManyEnabledMods
        | ModScriptError::TooManyScripts
        | ModScriptError::TooManyWorkspaceEntries
        | ModScriptError::WorkspaceTooLarge
        | ModScriptError::ModSetTooLarge
        | ModScriptError::SourceNotUtf8(_)
        | ModScriptError::ModSourceMissing(_) => {
            ModStudioError::new(ModStudioErrorCode::InvalidWorkspace, detail)
        }
    }
}

fn map_enabled_set_error(error: ModScriptError) -> ModStudioError {
    let detail = storage_error_detail(error.clone());
    match error {
        ModScriptError::FileSystem(_) => {
            ModStudioError::new(ModStudioErrorCode::EnabledSetWriteFailed, detail)
        }
        ModScriptError::InvalidModId(_) => {
            ModStudioError::new(ModStudioErrorCode::InvalidModId, detail)
        }
        ModScriptError::TooManyEnabledMods => {
            ModStudioError::new(ModStudioErrorCode::TooManyEnabledMods, detail)
        }
        ModScriptError::ModSourceMissing(_) => {
            ModStudioError::new(ModStudioErrorCode::ModSourceMissing, detail)
        }
        ModScriptError::InvalidModSet
        | ModScriptError::DuplicateModId(_)
        | ModScriptError::TooManyScripts
        | ModScriptError::TooManyWorkspaceEntries
        | ModScriptError::WorkspaceTooLarge
        | ModScriptError::ModSetTooLarge
        | ModScriptError::SourceTooLarge
        | ModScriptError::SourceContainsNul
        | ModScriptError::SourceNotUtf8(_)
        | ModScriptError::MissingVersionHeader
        | ModScriptError::MissingModDeclaration
        | ModScriptError::MismatchedModDeclaration
        | ModScriptError::MissingViewportTickHandler
        | ModScriptError::InvalidSourceLine(_)
        | ModScriptError::SourceBudgetExceeded
        | ModScriptError::CapabilityMismatch => {
            ModStudioError::new(ModStudioErrorCode::InvalidWorkspace, detail)
        }
    }
}

fn map_delete_error(error: ModScriptError) -> ModStudioError {
    let detail = storage_error_detail(error.clone());
    match error {
        ModScriptError::FileSystem(_) => {
            ModStudioError::new(ModStudioErrorCode::DeleteFailed, detail)
        }
        ModScriptError::InvalidModId(_) => {
            ModStudioError::new(ModStudioErrorCode::InvalidModId, detail)
        }
        ModScriptError::ModSourceMissing(_) => {
            ModStudioError::new(ModStudioErrorCode::ModSourceMissing, detail)
        }
        _ => ModStudioError::new(ModStudioErrorCode::InvalidWorkspace, detail),
    }
}

#[cfg(feature = "desktop")]
fn runtime_message_key(message: &str) -> Option<&'static str> {
    match message {
        "Hot reload applied." => Some("Hot reload applied."),
        "Mod workspace path is invalid; previous version kept." => {
            Some("Mod workspace path is invalid; previous version kept.")
        }
        "Enabled Mod set is invalid; previous version kept." => {
            Some("Enabled Mod set is invalid; previous version kept.")
        }
        "Enabled Mod set is unreadable; previous version kept." => {
            Some("Enabled Mod set is unreadable; previous version kept.")
        }
        "Mod source path is invalid; previous version kept." => {
            Some("Mod source path is invalid; previous version kept.")
        }
        "Enabled Mod source is missing; previous version kept." => {
            Some("Enabled Mod source is missing; previous version kept.")
        }
        "Compilation failed; previous version kept." => {
            Some("Compilation failed; previous version kept.")
        }
        "Runtime fault trapped; Mod paused until hot reload." => {
            Some("Runtime fault trapped; Mod paused until hot reload.")
        }
        _ => None,
    }
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
    fn enabled_binding_provider_is_discovered_from_mod_source() {
        let root = temp_workspace("binding-provider");
        let source = new_mod_script_template("provider").unwrap().replace(
            "NTE_MOD(\"provider\");",
            "NTE_MOD(\"provider\");\nNTE_BIND(\"button.future-action\");",
        );
        save_mod_script(&root, "provider", &source).expect("save provider");
        let service = ModStudioWorkspaceService::new(root.clone());

        assert_eq!(
            service
                .enabled_binding_provider("button.future-action")
                .expect("disabled lookup"),
            None
        );
        set_mod_enabled(&root, "provider", true).expect("enable provider");
        assert_eq!(
            service
                .enabled_binding_provider("button.future-action")
                .expect("enabled lookup")
                .as_deref(),
            Some("provider")
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
    fn creating_a_document_uses_the_shared_template_and_preserves_existing_files() {
        let root = temp_workspace("create");
        let service = ModStudioWorkspaceService::new(root.clone());

        let created = service.create_document("telemetry").expect("create Mod");
        assert_eq!(created.id, "telemetry");
        assert!(!created.enabled);
        assert_eq!(
            created.source,
            new_mod_script_template("telemetry").expect("template")
        );
        assert_eq!(service.load_workspace().expect("workspace").generation, 1);

        let error = service
            .create_document("telemetry")
            .expect_err("duplicate Mod");
        assert_eq!(error.code, ModStudioErrorCode::DocumentAlreadyExists);
        assert_eq!(
            service
                .load_document("telemetry")
                .expect("existing Mod")
                .source,
            created.source
        );

        fs::remove_dir_all(root).expect("remove workspace");
    }

    #[test]
    fn saving_valid_source_updates_the_existing_document() {
        let root = temp_workspace("save");
        let original = new_mod_script_template("telemetry").expect("template");
        save_mod_script(&root, "telemetry", &original).expect("initial save");
        let updated = format!("{original}// saved edit\n");

        let document =
            save_mod_studio_document(&root, "telemetry", &updated).expect("save document");

        assert_eq!(document.source, updated);
        assert_eq!(
            load_mod_studio_document(&root, "telemetry")
                .expect("reload")
                .source,
            updated
        );

        fs::remove_dir_all(root).expect("remove workspace");
    }

    #[test]
    fn rejected_source_preserves_the_previous_saved_file_and_original_line() {
        let root = temp_workspace("rejected-save");
        let original = new_mod_script_template("telemetry").expect("template");
        save_mod_script(&root, "telemetry", &original).expect("initial save");
        let invalid = original.replace(
            "nte::ipc::emit(\"post.session.changed\", character);",
            "this is not valid NTE C++;",
        );
        let invalid_line = invalid
            .lines()
            .position(|line| line.contains("this is not valid"))
            .expect("invalid line")
            + 1;

        let error =
            save_mod_studio_document(&root, "telemetry", &invalid).expect_err("reject source");

        assert_eq!(error.code, ModStudioErrorCode::InvalidSourceLine);
        assert_eq!(error.diagnostic_line, Some(invalid_line as u32));
        assert_eq!(
            load_mod_studio_document(&root, "telemetry")
                .expect("reload")
                .source,
            original
        );

        fs::remove_dir_all(root).expect("remove workspace");
    }

    #[test]
    fn enabled_set_changes_reuse_the_existing_workspace_transaction() {
        let root = temp_workspace("enabled-set");
        let source = new_mod_script_template("telemetry").expect("template");
        save_mod_script(&root, "telemetry", &source).expect("initial save");

        let enabled =
            set_mod_studio_document_enabled(&root, "telemetry", true).expect("enable Mod");
        assert!(enabled.documents[0].enabled);

        let disabled =
            set_mod_studio_document_enabled(&root, "telemetry", false).expect("disable Mod");
        assert!(!disabled.documents[0].enabled);

        fs::remove_dir_all(root).expect("remove workspace");
    }

    #[test]
    fn workspace_service_serializes_distinct_enabled_set_mutations() {
        let root = temp_workspace("enabled-set-service");
        for id in ["alpha", "beta"] {
            let source = new_mod_script_template(id).expect("template");
            save_mod_script(&root, id, &source).expect("initial save");
        }
        let service = ModStudioWorkspaceService::new(root.clone());
        let first_service = service.clone();
        let first = std::thread::spawn(move || {
            first_service
                .set_document_enabled("alpha", true)
                .expect("enable alpha")
                .generation
        });
        let second_service = service.clone();
        let second = std::thread::spawn(move || {
            second_service
                .set_document_enabled("beta", true)
                .expect("enable beta")
                .generation
        });

        let mut generations = [
            first.join().expect("alpha worker"),
            second.join().expect("beta worker"),
        ];
        generations.sort_unstable();
        let workspace = service.load_workspace().expect("final workspace");

        assert_eq!(generations, [1, 2]);
        assert_eq!(workspace.generation, 2);
        assert!(
            workspace
                .workspace
                .documents
                .iter()
                .all(|document| document.enabled)
        );

        fs::remove_dir_all(root).expect("remove workspace");
    }

    #[test]
    fn enabled_set_changes_require_an_existing_document() {
        let root = temp_workspace("missing-enable");
        fs::create_dir_all(&root).expect("create workspace");

        let error = set_mod_studio_document_enabled(&root, "missing", true)
            .expect_err("missing document is rejected");

        assert_eq!(error.code, ModStudioErrorCode::DocumentNotFound);
        assert!(!root.join("nte-mods.enabled").exists());

        fs::remove_dir_all(root).expect("remove workspace");
    }

    #[test]
    fn market_install_enables_the_bound_mod_in_the_same_workspace_transaction() {
        let root = temp_workspace("market-install");
        let service = ModStudioWorkspaceService::new(root.clone());
        let source = new_mod_script_template("sample").expect("market source");

        let installed = service
            .install_market_document("sample", &source)
            .expect("install market Mod");
        let workspace = service.load_workspace().expect("reload workspace");

        assert!(installed.enabled);
        assert_eq!(workspace.generation, 1);
        assert_eq!(workspace.workspace.documents.len(), 1);
        assert!(workspace.workspace.documents[0].enabled);
        fs::remove_dir_all(root).expect("remove workspace");
    }

    #[test]
    fn workspace_service_deletes_enabled_documents_and_advances_generation() {
        let root = temp_workspace("delete-document");
        let service = ModStudioWorkspaceService::new(root.clone());
        service.create_document("telemetry").unwrap();
        service.set_document_enabled("telemetry", true).unwrap();

        let deleted = service.delete_document("telemetry").unwrap();

        assert!(deleted.workspace.documents.is_empty());
        assert_eq!(deleted.generation, 3);
        assert_eq!(
            service.load_document("telemetry").unwrap_err().code,
            ModStudioErrorCode::DocumentNotFound
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "desktop")]
    #[test]
    fn runtime_status_projection_localizes_lifecycle_and_keeps_raw_script_logs() {
        use crate::platform::mods_plugin::{ModLogLevel, ModLogSnapshot};

        assert_eq!(
            runtime_message_key("Hot reload applied."),
            Some("Hot reload applied.")
        );
        assert_eq!(
            runtime_message_key("Runtime fault trapped; Mod paused until hot reload."),
            Some("Runtime fault trapped; Mod paused until hot reload.")
        );
        assert_eq!(runtime_message_key("user script output"), None);
        let raw = project_runtime_log(ModLogSnapshot {
            sequence: 7,
            timestamp_100ns: 70,
            mod_id: "telemetry".to_owned(),
            level: ModLogLevel::Info,
            message: "user script output".to_owned(),
        });
        assert_eq!(raw.message, "user script output");
        assert_eq!(raw.message_key, None);
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
