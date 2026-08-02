use serde::Serialize;

use nte_dps_tool::core::{
    mod_sdk::{MOD_SDK_SCHEMA_VERSION, ModSdkSymbol, ModSdkSymbolKind, mod_sdk_symbols},
    mod_studio::{
        MOD_STUDIO_WORKSPACE_LABEL, ModStudioDocument, ModStudioDocumentSummary, ModStudioError,
        ModStudioErrorCode, ModStudioRuntimeEvent as CoreModStudioRuntimeEvent,
        ModStudioRuntimeLevel, ModStudioRuntimeLog, VersionedModStudioWorkspace,
    },
};
use nte_dps_tool::platform::mods_plugin::{
    ModsPluginDeploymentError, ModsPluginDeploymentStatus, ModsPluginGameRegion,
    ModsPluginGameStatus,
};

use super::CommandError;

pub(crate) const MOD_STUDIO_CONTRACT_VERSION: u32 = 5;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModStudioWorkspaceSnapshot {
    pub contract_version: u32,
    pub generation: String,
    pub workspace_label: &'static str,
    pub documents: Vec<ModStudioDocumentSummarySnapshot>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModStudioDocumentSummarySnapshot {
    pub id: String,
    pub enabled: bool,
    pub source_bytes: u32,
    pub line_count: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModStudioDocumentSnapshot {
    pub contract_version: u32,
    pub id: String,
    pub enabled: bool,
    pub source: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModStudioSdkSchemaSnapshot {
    pub contract_version: u32,
    pub schema_version: u32,
    pub symbols: Vec<ModStudioSdkSymbolSnapshot>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModStudioSdkSymbolSnapshot {
    pub label: &'static str,
    pub insert_text: &'static str,
    pub kind: &'static str,
    pub return_type: Option<&'static str>,
    pub documentation_key: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModStudioRuntimeConnectionSnapshot {
    pub contract_version: u32,
    pub generation: String,
    pub connected: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModStudioRuntimeBatchSnapshot {
    pub contract_version: u32,
    pub generation: String,
    pub entries: Vec<ModStudioRuntimeEntrySnapshot>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum ModStudioRuntimeEntrySnapshot {
    Log {
        sequence: String,
        native_sequence: String,
        timestamp_100ns: String,
        mod_id: String,
        level: &'static str,
        message: String,
        message_key: Option<&'static str>,
        message_arguments: Vec<String>,
    },
    Event {
        sequence: String,
        native_sequence: String,
        timestamp_100ns: String,
        mod_id: String,
        name: String,
        values: Vec<String>,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", content = "payload", rename_all = "camelCase")]
pub(crate) enum ModStudioRuntimeEvent {
    Connection(ModStudioRuntimeConnectionSnapshot),
    Batch(ModStudioRuntimeBatchSnapshot),
}

impl From<VersionedModStudioWorkspace> for ModStudioWorkspaceSnapshot {
    fn from(versioned: VersionedModStudioWorkspace) -> Self {
        Self {
            contract_version: MOD_STUDIO_CONTRACT_VERSION,
            generation: versioned.generation.to_string(),
            workspace_label: MOD_STUDIO_WORKSPACE_LABEL,
            documents: versioned
                .workspace
                .documents
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ModStudioGameRegionSnapshot {
    China,
    Global,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModStudioGameStatusSnapshot {
    pub region: ModStudioGameRegionSnapshot,
    pub installed: bool,
    pub current: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModStudioDeploymentSnapshot {
    pub contract_version: u32,
    pub installations: u32,
    pub installed: u32,
    pub current: u32,
    pub source_available: bool,
    pub games: Vec<ModStudioGameStatusSnapshot>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModStudioDirectorySelectionSnapshot {
    pub selected: bool,
    pub path: Option<String>,
    pub deployment: ModStudioDeploymentSnapshot,
}

impl From<ModStudioDocumentSummary> for ModStudioDocumentSummarySnapshot {
    fn from(document: ModStudioDocumentSummary) -> Self {
        Self {
            id: document.id,
            enabled: document.enabled,
            source_bytes: document.source_bytes,
            line_count: document.line_count,
        }
    }
}

impl From<ModsPluginGameRegion> for ModStudioGameRegionSnapshot {
    fn from(region: ModsPluginGameRegion) -> Self {
        match region {
            ModsPluginGameRegion::China => Self::China,
            ModsPluginGameRegion::Global => Self::Global,
        }
    }
}

impl From<ModsPluginGameStatus> for ModStudioGameStatusSnapshot {
    fn from(status: ModsPluginGameStatus) -> Self {
        Self {
            region: status.region.into(),
            installed: status.installed,
            current: status.current,
        }
    }
}

impl From<ModsPluginDeploymentStatus> for ModStudioDeploymentSnapshot {
    fn from(status: ModsPluginDeploymentStatus) -> Self {
        Self {
            contract_version: MOD_STUDIO_CONTRACT_VERSION,
            installations: status.installations as u32,
            installed: status.installed as u32,
            current: status.current as u32,
            source_available: status.source_available,
            games: status.games.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<ModStudioDocument> for ModStudioDocumentSnapshot {
    fn from(document: ModStudioDocument) -> Self {
        Self {
            contract_version: MOD_STUDIO_CONTRACT_VERSION,
            id: document.id,
            enabled: document.enabled,
            source: document.source,
        }
    }
}

impl ModStudioSdkSchemaSnapshot {
    pub(crate) fn current() -> Self {
        Self {
            contract_version: MOD_STUDIO_CONTRACT_VERSION,
            schema_version: MOD_SDK_SCHEMA_VERSION,
            symbols: mod_sdk_symbols().map(Into::into).collect(),
        }
    }
}

impl From<ModSdkSymbol> for ModStudioSdkSymbolSnapshot {
    fn from(symbol: ModSdkSymbol) -> Self {
        let kind = match symbol.kind {
            ModSdkSymbolKind::Declaration => "declaration",
            ModSdkSymbolKind::Snippet => "snippet",
            ModSdkSymbolKind::Function => "function",
            ModSdkSymbolKind::Property => "property",
        };
        Self {
            label: symbol.label,
            insert_text: symbol.insert_text,
            kind,
            return_type: symbol.return_type,
            documentation_key: symbol.documentation_key,
        }
    }
}

impl ModStudioRuntimeEntrySnapshot {
    pub(crate) fn from_log(sequence: u64, log: ModStudioRuntimeLog) -> Self {
        let level = match log.level {
            ModStudioRuntimeLevel::Info => "info",
            ModStudioRuntimeLevel::Warning => "warning",
            ModStudioRuntimeLevel::Error => "error",
        };
        Self::Log {
            sequence: sequence.to_string(),
            native_sequence: log.sequence.to_string(),
            timestamp_100ns: log.timestamp_100ns.to_string(),
            mod_id: log.mod_id,
            level,
            message: log.message,
            message_key: log.message_key,
            message_arguments: log.message_arguments,
        }
    }

    pub(crate) fn from_event(sequence: u64, event: CoreModStudioRuntimeEvent) -> Self {
        Self::Event {
            sequence: sequence.to_string(),
            native_sequence: event.sequence.to_string(),
            timestamp_100ns: event.timestamp_100ns.to_string(),
            mod_id: event.mod_id,
            name: event.name,
            values: event
                .values
                .into_iter()
                .map(|value| value.to_string())
                .collect(),
        }
    }
}

impl CommandError {
    pub(crate) fn from_mod_studio(error: ModStudioError) -> Self {
        let (code, message_key) = match error.code {
            ModStudioErrorCode::FileSystem => (
                "mod_workspace_read_failed",
                "Failed to read the Mod workspace.",
            ),
            ModStudioErrorCode::WriteFailed => {
                ("mod_source_write_failed", "Failed to save the Mod source.")
            }
            ModStudioErrorCode::EnabledSetWriteFailed => (
                "mod_enabled_set_write_failed",
                "Failed to update the enabled Mod set.",
            ),
            ModStudioErrorCode::InvalidWorkspace => (
                "mod_workspace_invalid",
                "The Mod workspace data is invalid.",
            ),
            ModStudioErrorCode::InvalidModId => {
                ("invalid_mod_id", "The Mod identifier is invalid.")
            }
            ModStudioErrorCode::DocumentAlreadyExists => (
                "mod_document_already_exists",
                "A Mod with this ID already exists.",
            ),
            ModStudioErrorCode::DocumentNotFound => (
                "mod_document_not_found",
                "The selected Mod document no longer exists.",
            ),
            ModStudioErrorCode::SourceTooLarge => (
                "mod_source_too_large",
                "A Mod source file can contain at most 16 KiB.",
            ),
            ModStudioErrorCode::SourceContainsNul => (
                "mod_source_contains_nul",
                "A Mod source file cannot contain NUL bytes.",
            ),
            ModStudioErrorCode::MissingVersionHeader => (
                "mod_source_missing_version",
                "The first statement must be NTE_SCRIPT(5).",
            ),
            ModStudioErrorCode::MissingModDeclaration => (
                "mod_source_missing_declaration",
                "The script must declare NTE_MOD(\"id\").",
            ),
            ModStudioErrorCode::MismatchedModDeclaration => (
                "mod_source_mismatched_declaration",
                "The NTE_MOD(\"id\") declaration must match the file name.",
            ),
            ModStudioErrorCode::MissingViewportTickHandler => (
                "mod_source_missing_viewport_tick",
                "The script must define on_viewport_tick(event).",
            ),
            ModStudioErrorCode::InvalidSourceLine => (
                "mod_source_invalid_line",
                "The NTE C++ compiler rejected line {0}.",
            ),
            ModStudioErrorCode::SourceBudgetExceeded => (
                "mod_source_budget_exceeded",
                "The NTE C++ program exceeds the compiler resource budget.",
            ),
            ModStudioErrorCode::CapabilityMismatch => (
                "mod_source_capability_mismatch",
                "Declared Mod capabilities must exactly match the APIs used by the script.",
            ),
            ModStudioErrorCode::TooManyEnabledMods => (
                "too_many_enabled_mods",
                "At most 16 Mods can be enabled at the same time.",
            ),
            ModStudioErrorCode::ModSourceMissing => (
                "mod_source_missing",
                "The selected Mod source file no longer exists.",
            ),
        };
        let message_arguments = match error.diagnostic_line {
            Some(line) => vec![line.to_string()],
            None => Vec::new(),
        };
        Self {
            code,
            message_key,
            message_arguments,
            diagnostic_line: error.diagnostic_line,
        }
    }

    pub(crate) fn mod_workspace_task_failed() -> Self {
        Self {
            code: "mod_workspace_task_failed",
            message_key: "The Mod workspace task stopped unexpectedly.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn from_mod_studio_deployment(error: ModsPluginDeploymentError) -> Self {
        let (code, message_key) = match error {
            ModsPluginDeploymentError::GameRunning => (
                "mod_loader_game_running",
                "Close HTGame.exe before changing the Mod loader.",
            ),
            ModsPluginDeploymentError::GameProcessProbe(_) => (
                "mod_loader_game_probe_failed",
                "Failed to check whether HTGame.exe is running.",
            ),
            ModsPluginDeploymentError::GameInstallationNotFound => (
                "mod_loader_game_not_found",
                "Game installation not detected",
            ),
            ModsPluginDeploymentError::InvalidGameDirectory => (
                "mod_loader_game_directory_invalid",
                "The selected folder does not contain HTGame.exe.",
            ),
            ModsPluginDeploymentError::Registry(_) => (
                "mod_loader_registry_failed",
                "Failed to locate the game installation from the registry.",
            ),
            ModsPluginDeploymentError::PluginSourceNotFound => (
                "mod_loader_source_not_found",
                "Mod loader file plugins/dwmapi.dll was not found",
            ),
            ModsPluginDeploymentError::ConflictingDwmapi => (
                "mod_loader_conflict",
                "The game directory already contains an unmanaged dwmapi.dll.",
            ),
            ModsPluginDeploymentError::InstalledPluginChanged => (
                "mod_loader_changed",
                "The installed dwmapi.dll was replaced outside this tool.",
            ),
            ModsPluginDeploymentError::FileSystem(_) => (
                "mod_loader_file_failed",
                "Failed to update the Mod loader files.",
            ),
        };
        Self {
            code,
            message_key,
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn mod_studio_file_dialog_failed() -> Self {
        Self {
            code: "mod_studio_file_dialog_failed",
            message_key: "The native file dialog failed",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn mod_studio_folder_open_failed() -> Self {
        Self {
            code: "mod_studio_folder_open_failed",
            message_key: "Failed to open the Mod folder.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn invalid_mod_runtime_subscription_id() -> Self {
        Self {
            code: "invalid_mod_runtime_subscription_id",
            message_key: "The Mod runtime subscription identifier is invalid.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_snapshot_uses_a_relative_label_and_bounded_numbers() {
        let snapshot = ModStudioWorkspaceSnapshot::from(VersionedModStudioWorkspace {
            generation: 7,
            workspace: nte_dps_tool::core::mod_studio::ModStudioWorkspace {
                documents: vec![ModStudioDocumentSummary {
                    id: "telemetry".to_owned(),
                    enabled: true,
                    source_bytes: 1024,
                    line_count: 40,
                }],
            },
        });
        let value = serde_json::to_value(snapshot).expect("serialize workspace");

        assert_eq!(value["contractVersion"], MOD_STUDIO_CONTRACT_VERSION);
        assert_eq!(value["generation"], "7");
        assert_eq!(value["workspaceLabel"], "plugins/nte-mods");
        assert_eq!(value["documents"][0]["sourceBytes"], 1024);
        assert_eq!(value["documents"][0]["lineCount"], 40);
        assert!(value.to_string().find(":\\\\Users\\\\").is_none());
    }

    #[test]
    fn document_snapshot_keeps_source_on_the_detail_command_only() {
        let snapshot = ModStudioDocumentSnapshot::from(ModStudioDocument {
            id: "telemetry".to_owned(),
            enabled: false,
            source: "NTE_SCRIPT(5);".to_owned(),
        });
        let value = serde_json::to_value(snapshot).expect("serialize document");

        assert_eq!(value["id"], "telemetry");
        assert_eq!(value["source"], "NTE_SCRIPT(5);");
    }

    #[test]
    fn deployment_snapshot_uses_stable_regions_and_omits_game_paths() {
        let snapshot = ModStudioDeploymentSnapshot::from(ModsPluginDeploymentStatus {
            installations: 1,
            installed: 1,
            current: 1,
            source_available: true,
            games: vec![ModsPluginGameStatus {
                region: ModsPluginGameRegion::Global,
                installed: true,
                current: true,
            }],
        });
        let value = serde_json::to_value(snapshot).expect("serialize deployment");

        assert_eq!(value["contractVersion"], MOD_STUDIO_CONTRACT_VERSION);
        assert_eq!(value["games"][0]["region"], "global");
        assert!(value.get("path").is_none());
    }

    #[test]
    fn sdk_schema_projects_the_single_rust_symbol_source() {
        let value =
            serde_json::to_value(ModStudioSdkSchemaSnapshot::current()).expect("serialize schema");

        assert_eq!(value["contractVersion"], MOD_STUDIO_CONTRACT_VERSION);
        assert_eq!(value["schemaVersion"], MOD_SDK_SCHEMA_VERSION);
        assert_eq!(value["symbols"].as_array().map(Vec::len), Some(88));
        assert!(value["symbols"].as_array().is_some_and(|symbols| {
            symbols.iter().any(|symbol| {
                symbol["label"] == "nte::memory::read_ptr(base, offset)"
                    && symbol["returnType"] == "std::uintptr_t"
            })
        }));
    }

    #[test]
    fn core_error_categories_map_to_stable_command_errors() {
        let error = CommandError::from_mod_studio(ModStudioError {
            code: ModStudioErrorCode::DocumentNotFound,
            detail: "internal path detail".to_owned(),
            diagnostic_line: None,
        });
        let value = serde_json::to_value(error).expect("serialize error");

        assert_eq!(value["code"], "mod_document_not_found");
        assert_eq!(
            value["messageKey"],
            "The selected Mod document no longer exists."
        );
        assert!(!value.to_string().contains("internal path detail"));
    }

    #[test]
    fn source_line_diagnostic_keeps_the_original_line_number() {
        let error = CommandError::from_mod_studio(ModStudioError {
            code: ModStudioErrorCode::InvalidSourceLine,
            detail: "internal compiler detail".to_owned(),
            diagnostic_line: Some(23),
        });
        let value = serde_json::to_value(error).expect("serialize error");

        assert_eq!(value["code"], "mod_source_invalid_line");
        assert_eq!(value["diagnosticLine"], 23);
        assert_eq!(value["messageArguments"][0], "23");
        assert!(!value.to_string().contains("internal compiler detail"));
    }

    #[test]
    fn runtime_entries_keep_stream_and_native_u64_values_as_decimal_strings() {
        let snapshot = ModStudioRuntimeEntrySnapshot::from_log(
            u64::MAX,
            ModStudioRuntimeLog {
                sequence: u64::MAX,
                timestamp_100ns: u64::MAX - 1,
                mod_id: "runtime".to_owned(),
                level: ModStudioRuntimeLevel::Warning,
                message: "Compilation failed; previous version kept.".to_owned(),
                message_key: Some("Compilation failed; previous version kept."),
                message_arguments: Vec::new(),
            },
        );
        let value = serde_json::to_value(snapshot).expect("serialize runtime entry");

        assert_eq!(value["sequence"], u64::MAX.to_string());
        assert_eq!(value["nativeSequence"], u64::MAX.to_string());
        assert_eq!(value["timestamp100ns"], (u64::MAX - 1).to_string());
        assert_eq!(value["level"], "warning");
        assert_eq!(value["kind"], "log");
    }

    #[test]
    fn runtime_events_keep_values_as_decimal_strings() {
        let snapshot = ModStudioRuntimeEntrySnapshot::from_event(
            u64::MAX - 1,
            CoreModStudioRuntimeEvent {
                sequence: u64::MAX,
                timestamp_100ns: u64::MAX - 2,
                mod_id: "telemetry".to_owned(),
                name: "post.sample".to_owned(),
                values: vec![0, u64::MAX],
            },
        );
        let value = serde_json::to_value(snapshot).expect("serialize runtime event");

        assert_eq!(value["kind"], "event");
        assert_eq!(value["sequence"], (u64::MAX - 1).to_string());
        assert_eq!(value["nativeSequence"], u64::MAX.to_string());
        assert_eq!(value["timestamp100ns"], (u64::MAX - 2).to_string());
        assert_eq!(value["values"][0], "0");
        assert_eq!(value["values"][1], u64::MAX.to_string());
    }

    #[test]
    fn enabled_set_errors_use_stable_messages_without_storage_detail() {
        let error = CommandError::from_mod_studio(ModStudioError {
            code: ModStudioErrorCode::TooManyEnabledMods,
            detail: "private enabled-set detail".to_owned(),
            diagnostic_line: None,
        });
        let value = serde_json::to_value(error).expect("serialize error");

        assert_eq!(value["code"], "too_many_enabled_mods");
        assert_eq!(
            value["messageKey"],
            "At most 16 Mods can be enabled at the same time."
        );
        assert!(!value.to_string().contains("private enabled-set detail"));
    }
}
