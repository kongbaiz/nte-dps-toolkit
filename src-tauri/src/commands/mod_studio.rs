use tauri::WebviewWindow;

use nte_dps_tool::core::mod_studio::{
    load_default_mod_studio_document, load_default_mod_studio_workspace,
};

use crate::{
    contract::{
        CommandError,
        mod_studio::{ModStudioDocumentSnapshot, ModStudioWorkspaceSnapshot},
    },
    windows::console,
};

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
