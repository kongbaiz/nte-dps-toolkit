use tauri::WebviewWindow;

use nte_dps_tool::core::mod_studio::{
    load_default_mod_studio_document, load_default_mod_studio_workspace,
    save_default_mod_studio_document, set_default_mod_studio_document_enabled,
};

use crate::{
    contract::{
        CommandError,
        mod_studio::{
            ModStudioDocumentSnapshot, ModStudioSdkSchemaSnapshot, ModStudioWorkspaceSnapshot,
        },
    },
    windows::console,
};

#[tauri::command]
pub(crate) fn get_mod_studio_sdk_schema(
    window: WebviewWindow,
) -> Result<ModStudioSdkSchemaSnapshot, CommandError> {
    console::validate_window(&window)?;
    Ok(ModStudioSdkSchemaSnapshot::current())
}

#[tauri::command]
pub(crate) async fn get_mod_studio_workspace(
    window: WebviewWindow,
) -> Result<ModStudioWorkspaceSnapshot, CommandError> {
    console::validate_window(&window)?;
    tauri::async_runtime::spawn_blocking(load_default_mod_studio_workspace)
        .await
        .map_err(|error| {
            log::error!("Mod workspace list task failed: {error}");
            CommandError::mod_workspace_task_failed()
        })?
        .map(Into::into)
        .map_err(|error| {
            log::error!("Mod workspace list failed: {}", error.detail);
            CommandError::from_mod_studio(error)
        })
}

#[tauri::command]
pub(crate) async fn get_mod_studio_document(
    id: String,
    window: WebviewWindow,
) -> Result<ModStudioDocumentSnapshot, CommandError> {
    console::validate_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || load_default_mod_studio_document(&id))
        .await
        .map_err(|error| {
            log::error!("Mod document task failed: {error}");
            CommandError::mod_workspace_task_failed()
        })?
        .map(Into::into)
        .map_err(|error| {
            log::error!("Mod document load failed: {}", error.detail);
            CommandError::from_mod_studio(error)
        })
}

#[tauri::command]
pub(crate) async fn save_mod_studio_document(
    id: String,
    source: String,
    window: WebviewWindow,
) -> Result<ModStudioDocumentSnapshot, CommandError> {
    console::validate_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || save_default_mod_studio_document(&id, &source))
        .await
        .map_err(|error| {
            log::error!("Mod document save task failed: {error}");
            CommandError::mod_workspace_task_failed()
        })?
        .map(Into::into)
        .map_err(|error| {
            log::error!("Mod document save failed: {}", error.detail);
            CommandError::from_mod_studio(error)
        })
}

#[tauri::command]
pub(crate) async fn set_mod_studio_document_enabled(
    id: String,
    enabled: bool,
    window: WebviewWindow,
) -> Result<ModStudioWorkspaceSnapshot, CommandError> {
    console::validate_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        set_default_mod_studio_document_enabled(&id, enabled)
    })
    .await
    .map_err(|error| {
        log::error!("Mod enabled-set task failed: {error}");
        CommandError::mod_workspace_task_failed()
    })?
    .map(Into::into)
    .map_err(|error| {
        log::error!("Mod enabled-set update failed: {}", error.detail);
        CommandError::from_mod_studio(error)
    })
}
