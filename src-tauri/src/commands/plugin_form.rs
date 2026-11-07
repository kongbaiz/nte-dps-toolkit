use super::toolkit::{ReleaseResult, client, preview};
use crate::{contract::CommandError, state::AppState, windows::console};
use nte_dps_tool::{
    core::{live_capture::LiveCapturePhase, toolkit::DataMode},
    platform::toolkit::{
        ToolkitError,
        form::{self, PluginFormSnapshot},
    },
};
use serde::Deserialize;
use serde_json::Value;
use tauri::{State, WebviewWindow};
use ts_rs::TS;

#[tauri::command]
pub(crate) async fn get_plugin_form(
    file: String,
    expected_identity: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<PluginFormSnapshot, CommandError> {
    console::validate_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = state.reserve_plugin_control()?;
        if state.data_mode() != DataMode::Plugin {
            return Err(ToolkitError::Unsupported.into());
        }
        let (host, description) = client()?;
        if description.operation_session != expected_identity {
            return Err(ToolkitError::SessionChanged.into());
        }
        Ok(form::read(&host, &description, &file)?)
    })
    .await
    .map_err(|_| CommandError::from(ToolkitError::Failed))?
}

#[derive(Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginFormActionRequest {
    file: String,
    expected_identity: String,
    expected_owner: String,
    action: String,
    #[ts(type = "unknown")]
    data: Value,
    confirmed: bool,
}

#[tauri::command]
pub(crate) async fn invoke_plugin_form_action(
    request: PluginFormActionRequest,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<ReleaseResult, CommandError> {
    let PluginFormActionRequest {
        file,
        expected_identity,
        expected_owner,
        action,
        data,
        confirmed,
    } = request;
    console::validate_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = state.reserve_plugin_control()?;
        if state.data_mode() != DataMode::Plugin {
            return Err(ToolkitError::Unsupported.into());
        }
        let (host, description) = client()?;
        if description.operation_session != expected_identity {
            return Err(ToolkitError::SessionChanged.into());
        }
        let current = form::read(&host, &description, &file)?;
        if current.owner != expected_owner {
            return Err(ToolkitError::SessionChanged.into());
        }
        let action = current
            .form
            .actions
            .iter()
            .find(|item| item.id == action)
            .ok_or(ToolkitError::Unsupported)?;
        if action.confirmation.is_some() && !confirmed {
            return Err(CommandError::mod_studio_risk_acknowledgement_required());
        }
        if action.requires_idle
            && matches!(
                state.capture_phase(),
                LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
            )
        {
            return Err(ToolkitError::Busy.into());
        }
        let bytes = action.invoke(&host, &description.operation_session, &current.owner, &data)?;
        preview(action.command, &bytes).map_err(CommandError::from)
    })
    .await
    .map_err(|_| CommandError::from(ToolkitError::Failed))?
}
