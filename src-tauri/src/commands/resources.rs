use nte_dps_tool::core::resource_audit::audit_runtime_resources;
use tauri::WebviewWindow;

use crate::{
    contract::{CommandError, resources::ResourcesSnapshot},
    windows::console,
};

#[tauri::command]
pub(crate) async fn get_resources_snapshot(
    window: WebviewWindow,
) -> Result<ResourcesSnapshot, CommandError> {
    console::validate_window(&window)?;
    tauri::async_runtime::spawn_blocking(audit_runtime_resources)
        .await
        .map(ResourcesSnapshot::from_summary)
        .map_err(|_| {
            CommandError::resources(
                "resource_audit_failed",
                "Runtime resource check did not finish.",
            )
        })
}
