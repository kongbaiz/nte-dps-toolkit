use serde::Serialize;

use nte_dps_tool::core::mod_studio::{
    MOD_STUDIO_WORKSPACE_LABEL, ModStudioDocument, ModStudioDocumentSummary, ModStudioError,
    ModStudioErrorCode, ModStudioWorkspace,
};

use super::CommandError;

pub(crate) const MOD_STUDIO_CONTRACT_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModStudioWorkspaceSnapshot {
    pub contract_version: u32,
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

impl From<ModStudioWorkspace> for ModStudioWorkspaceSnapshot {
    fn from(workspace: ModStudioWorkspace) -> Self {
        Self {
            contract_version: MOD_STUDIO_CONTRACT_VERSION,
            workspace_label: MOD_STUDIO_WORKSPACE_LABEL,
            documents: workspace.documents.into_iter().map(Into::into).collect(),
        }
    }
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

impl CommandError {
    pub(crate) fn from_mod_studio(error: ModStudioError) -> Self {
        let (code, message_key) = match error.code {
            ModStudioErrorCode::FileSystem => (
                "mod_workspace_read_failed",
                "Failed to read the Mod workspace.",
            ),
            ModStudioErrorCode::InvalidWorkspace => (
                "mod_workspace_invalid",
                "The Mod workspace data is invalid.",
            ),
            ModStudioErrorCode::InvalidModId => {
                ("invalid_mod_id", "The Mod identifier is invalid.")
            }
            ModStudioErrorCode::DocumentNotFound => (
                "mod_document_not_found",
                "The selected Mod document no longer exists.",
            ),
        };
        Self {
            code,
            message_key,
            message_arguments: Vec::new(),
        }
    }

    pub(crate) fn mod_workspace_task_failed() -> Self {
        Self {
            code: "mod_workspace_task_failed",
            message_key: "The Mod workspace task stopped unexpectedly.",
            message_arguments: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_snapshot_uses_a_relative_label_and_bounded_numbers() {
        let snapshot = ModStudioWorkspaceSnapshot::from(ModStudioWorkspace {
            documents: vec![ModStudioDocumentSummary {
                id: "telemetry".to_owned(),
                enabled: true,
                source_bytes: 1024,
                line_count: 40,
            }],
        });
        let value = serde_json::to_value(snapshot).expect("serialize workspace");

        assert_eq!(value["contractVersion"], MOD_STUDIO_CONTRACT_VERSION);
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
    fn core_error_categories_map_to_stable_command_errors() {
        let error = CommandError::from_mod_studio(ModStudioError {
            code: ModStudioErrorCode::DocumentNotFound,
            detail: "internal path detail".to_owned(),
        });
        let value = serde_json::to_value(error).expect("serialize error");

        assert_eq!(value["code"], "mod_document_not_found");
        assert_eq!(
            value["messageKey"],
            "The selected Mod document no longer exists."
        );
        assert!(!value.to_string().contains("internal path detail"));
    }
}
