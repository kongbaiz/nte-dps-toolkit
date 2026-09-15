use crate::{
    contract::CommandError,
    file_dialog::{self, DialogOutcome},
    state::AppState,
    windows::console,
};
use nte_dps_tool::{
    core::toolkit::{CombatStatus, DataMode},
    platform::{
        network,
        toolkit::{ToolkitClient, ToolkitError},
    },
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{State, WebviewWindow};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginPanel {
    contract_version: u32,
    mode: DataMode,
    connection: &'static str,
    plugins: Vec<Plugin>,
    combat: Option<Combat>,
    operation: Option<PluginOperation>,
    directory_selected: bool,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Plugin {
    file: String,
    state: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Combat {
    generation: String,
    capturing: bool,
    hits: String,
    total_damage: f64,
    direct: f64,
    correlated: f64,
    inferred: f64,
    unknown: f64,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct PluginOperation {
    state: String,
    error: String,
}

fn sanitize_operation(mut operation: PluginOperation) -> Result<PluginOperation, CommandError> {
    if !matches!(
        operation.state.as_str(),
        "" | "queued" | "pending" | "completed" | "failed" | "rejected"
    ) || operation.error.len() > 512
    {
        return Err(error(ToolkitError::InvalidProtocol));
    }
    // Native diagnostics can contain game-directory paths. Expose a stable key only.
    if !operation.error.is_empty() {
        operation.error = "The plugin operation failed. Query its status before retrying.".into();
    }
    Ok(operation)
}

pub(crate) fn unsupported() -> CommandError {
    error(ToolkitError::Unsupported)
}

fn error(e: ToolkitError) -> CommandError {
    let (code, key) = match e {
        ToolkitError::Unavailable => (
            "plugin_unavailable",
            "UE Tools is not connected. Load the compiled Toolkit package in the game first.",
        ),
        ToolkitError::Busy => (
            "plugin_busy",
            "The plugin is busy. Query its status before retrying.",
        ),
        ToolkitError::Timeout => (
            "plugin_timeout",
            "The plugin operation timed out. Its result is unknown; do not retry automatically.",
        ),
        ToolkitError::Unsupported => (
            "plugin_unsupported",
            "This feature is not supported by the connected plugin.",
        ),
        _ => (
            "plugin_invalid",
            "Plugin data is unavailable or invalid. Packet capture was not started.",
        ),
    };
    CommandError {
        code,
        message_key: key,
        message_arguments: vec![],
        diagnostic_line: None,
    }
}
fn client() -> Result<ToolkitClient, CommandError> {
    let pid = network::game_process_id()
        .map_err(|_| error(ToolkitError::Failed))?
        .ok_or_else(|| error(ToolkitError::Unavailable))?;
    let c = ToolkitClient::open(pid).map_err(error)?;
    c.describe().map_err(error)?;
    Ok(c)
}
pub(crate) fn plugin_directory(state: &AppState) -> Result<PathBuf, CommandError> {
    let root = state
        .mod_studio_game_directory(nte_dps_tool::platform::mods_plugin::ModsPluginGameRegion::China)
        .ok_or_else(|| error(ToolkitError::Unavailable))?;
    let root = PathBuf::from(root);
    // Only an explicitly selected, already installed Toolkit host directory is accepted.
    if !root.join("HTGame.exe").is_file()
        || !root.join("d3d12.dll").is_file()
        || root.join("dwmapi.dll").exists()
    {
        return Err(error(ToolkitError::Unsupported));
    }
    Ok(root.join("plugins"))
}
fn snapshot(state: &AppState) -> PluginPanel {
    let mode = state.data_mode();
    let mut result = PluginPanel {
        contract_version: 1,
        mode,
        connection: "notRequested",
        plugins: vec![],
        combat: None,
        operation: None,
        directory_selected: plugin_directory(state).is_ok(),
    };
    // Capture mode never probes or initializes any plugin IPC.
    if mode == DataMode::PacketCapture {
        return result;
    }
    let read = (|| {
        let c = client()?;
        let plugins: Vec<Plugin> = c.json(3, 0, "", &|| false).map_err(error)?;
        if plugins.len() > 64
            || plugins.iter().any(|p| {
                !valid_file(&p.file)
                    || !matches!(
                        p.state.as_str(),
                        "loaded" | "unloaded" | "unload_pending" | "failed"
                    )
            })
        {
            return Err(error(ToolkitError::InvalidProtocol));
        }
        let combat: CombatStatus = c.json(100, 0, "", &|| false).map_err(error)?;
        if combat.encounter_id.len() > 256
            || [
                combat.total_damage,
                combat.quality.direct,
                combat.quality.correlated,
                combat.quality.inferred,
                combat.quality.unknown,
            ]
            .iter()
            .any(|x| !x.is_finite() || *x < 0.0)
        {
            return Err(error(ToolkitError::InvalidProtocol));
        }
        let operation = sanitize_operation(c.json(7, 0, "", &|| false).map_err(error)?)?;
        Ok((plugins, combat, operation))
    })();
    match read {
        Ok((plugins, s, op)) => {
            result.connection = "connected";
            result.plugins = plugins;
            result.operation = Some(op);
            result.combat = Some(Combat {
                generation: s.capture_generation.to_string(),
                capturing: s.capturing,
                hits: s.hits.to_string(),
                total_damage: s.total_damage,
                direct: s.quality.direct,
                correlated: s.quality.correlated,
                inferred: s.quality.inferred,
                unknown: s.quality.unknown,
            });
        }
        Err(e) => {
            result.connection = match e.code {
                "plugin_unavailable" => "unavailable",
                "plugin_busy" => "busy",
                _ => "error",
            }
        }
    }
    result
}
fn valid_file(file: &str) -> bool {
    file.len() <= 128
        && file.ends_with(".dll")
        && file
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        && !file.contains("..")
}

#[tauri::command]
pub(crate) async fn get_plugin_panel(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<PluginPanel, CommandError> {
    console::validate_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || snapshot(&state))
        .await
        .map_err(|_| error(ToolkitError::Failed))
}
#[tauri::command]
pub(crate) async fn set_data_mode(
    mode: DataMode,
    acknowledge_risk: bool,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<PluginPanel, CommandError> {
    console::validate_window(&window)?;
    if mode == DataMode::Plugin && !acknowledge_risk {
        return Err(CommandError::mod_studio_risk_acknowledgement_required());
    }
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.set_data_mode(mode).map_err(CommandError::from_core)?;
        // Choosing a mode does not discard a recorded session. The normal Start
        // command retains its existing replace-current confirmation contract.
        Ok(snapshot(&state))
    })
    .await
    .map_err(|_| error(ToolkitError::Failed))?
}
#[tauri::command]
pub(crate) async fn control_plugin(
    action: String,
    file: Option<String>,
    value: Option<u32>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    console::validate_window(&window)?;
    if state.data_mode() != DataMode::Plugin {
        return Err(CommandError::mod_studio_risk_acknowledgement_required());
    }
    let file = file.unwrap_or_default();
    let (command, argument) = match action.as_str() {
        "enable" | "disable" | "reload" if valid_file(&file) => (
            match action.as_str() {
                "enable" => 4,
                "disable" => 5,
                _ => 6,
            },
            0,
        ),
        "logLevel" if value.is_some_and(|v| v <= 6) => (8, value.unwrap_or(0)),
        _ => return Err(error(ToolkitError::Unsupported)),
    };
    // Combat collector owns start/stop. Never unload it under an active stream.
    if matches!(
        state.capture_phase(),
        nte_dps_tool::core::live_capture::LiveCapturePhase::Starting
            | nte_dps_tool::core::live_capture::LiveCapturePhase::Running
            | nte_dps_tool::core::live_capture::LiveCapturePhase::Stopping
    ) && command != 8
    {
        return Err(error(ToolkitError::Busy));
    }
    tauri::async_runtime::spawn_blocking(move || {
        client()?
            .call(command, argument, &file, &|| false)
            .map_err(error)?;
        Ok(())
    })
    .await
    .map_err(|_| error(ToolkitError::Failed))?
}
#[tauri::command]
pub(crate) async fn select_plugin_directory(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<bool, CommandError> {
    console::validate_window(&window)?;
    let selected = file_dialog::choose_folder(
        &window,
        nte_dps_tool::storage::i18n::t(
            "Select the game folder containing HTGame.exe and d3d12.dll",
        ),
    )
    .await
    .map_err(|_| error(ToolkitError::Failed))?;
    let DialogOutcome::Selected(root) = selected else {
        return Ok(false);
    };
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if !root.join("HTGame.exe").is_file()
            || !root.join("d3d12.dll").is_file()
            || root.join("dwmapi.dll").exists()
        {
            return Err(error(ToolkitError::Unsupported));
        }
        state
            .set_mod_studio_game_directory(
                nte_dps_tool::platform::mods_plugin::ModsPluginGameRegion::China,
                Some(root.to_string_lossy().into_owned()),
            )
            .map_err(|_| CommandError::settings_config_save_failed())?;
        Ok(true)
    })
    .await
    .map_err(|_| error(ToolkitError::Failed))?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_operation_diagnostics_never_expose_private_paths() {
        let operation = sanitize_operation(PluginOperation {
            state: "failed".into(),
            error: "failed to open PRIVATE_PATH".into(),
        })
        .unwrap();
        assert_eq!(
            operation.error,
            "The plugin operation failed. Query its status before retrying."
        );
        assert!(
            sanitize_operation(PluginOperation {
                state: "PRIVATE_PATH".into(),
                error: String::new()
            })
            .is_err()
        );
    }
    #[test]
    fn plugin_control_never_accepts_paths_or_scripts() {
        assert!(valid_file("NTE_PluginCombat.dll"));
        for file in [
            "../a.dll", "a/b.dll", "a\\b.dll", "a.nte", "C:a.dll", "..dll",
        ] {
            assert!(!valid_file(file));
        }
    }
}
