use crate::{
    contract::CommandError,
    state::AppState,
    windows::{console, main_dps},
};
use nte_dps_tool::{
    core::toolkit::{CombatStatus, DataMode},
    platform::{
        network,
        toolkit::{ToolkitClient, ToolkitError},
    },
    storage::config::ModStudioLoadingMethod,
};
use serde::{Deserialize, Serialize};
use std::{io::Read, path::PathBuf};
use tauri::{State, WebviewWindow};

const PLUGIN_PANEL_CONTRACT_VERSION: u32 = 5;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginPanel {
    contract_version: u32,
    connection_identity: Option<String>,
    collector_active: bool,
    capabilities: Vec<u32>,
    mode: DataMode,
    loading_method: ModStudioLoadingMethod,
    connection: &'static str,
    plugins: Vec<Plugin>,
    combat: Option<Combat>,
    operation: Option<PluginOperation>,
    components_ready: bool,
    game_directory: Option<String>,
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

pub(super) fn error(e: ToolkitError) -> CommandError {
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
pub(super) fn client() -> Result<(ToolkitClient, Vec<u32>), CommandError> {
    let pid = network::game_process_id()
        .map_err(|_| error(ToolkitError::Failed))?
        .ok_or_else(|| error(ToolkitError::Unavailable))?;
    let c = ToolkitClient::open(pid).map_err(error)?;
    let capabilities = c.capabilities().map_err(error)?;
    Ok((c, capabilities))
}
fn runtime_directory(
    root: &std::path::Path,
    method: ModStudioLoadingMethod,
) -> Result<PathBuf, CommandError> {
    let runtime = root.join("game");
    if method == ModStudioLoadingMethod::Loader
        && (!root.join("tools/NTE-Loader.exe").is_file()
            || !root.join("driver/uetools.sys").is_file())
    {
        return Err(error(ToolkitError::Unsupported));
    }
    if !runtime.join("d3d12.dll").is_file() || runtime.join("dwmapi.dll").exists() {
        return Err(error(ToolkitError::Unsupported));
    }
    Ok(runtime)
}

fn validate_data_source_window(window: &WebviewWindow) -> Result<(), CommandError> {
    if matches!(
        window.label(),
        console::CONSOLE_WINDOW_LABEL | main_dps::MAIN_DPS_WINDOW_LABEL
    ) {
        Ok(())
    } else {
        Err(CommandError::invalid_window())
    }
}

fn snapshot(state: &AppState) -> PluginPanel {
    let mode = state.data_mode();
    let mut result = PluginPanel {
        contract_version: PLUGIN_PANEL_CONTRACT_VERSION,
        game_directory: state.proxy_game_directory(),
        connection_identity: None,
        collector_active: matches!(
            state.capture_phase(),
            nte_dps_tool::core::live_capture::LiveCapturePhase::Starting
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Running
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Stopping
        ),
        capabilities: vec![],
        mode,
        loading_method: state.mod_studio_loading_method(),
        connection: "notRequested",
        plugins: vec![],
        combat: None,
        operation: None,
        components_ready: runtime_directory(
            &nte_dps_tool::storage::paths::toolkit_dir(),
            state.mod_studio_loading_method(),
        )
        .is_ok(),
    };
    // Capture mode never probes or initializes any plugin IPC.
    if mode == DataMode::PacketCapture {
        return result;
    }
    let read = (|| {
        let (c, capabilities) = client()?;
        let identity = c.identity();
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
        let combat: Option<CombatStatus> = match c.json(100, 0, "", &|| false) {
            Ok(value) => Some(value),
            Err(ToolkitError::Unavailable | ToolkitError::Unsupported) => None,
            Err(e) => return Err(error(e)),
        };
        if let Some(combat) = &combat
            && (combat.encounter_id.len() > 256
                || [
                    combat.total_damage,
                    combat.quality.direct,
                    combat.quality.correlated,
                    combat.quality.inferred,
                    combat.quality.unknown,
                ]
                .iter()
                .any(|x| !x.is_finite() || *x < 0.0))
        {
            return Err(error(ToolkitError::InvalidProtocol));
        }
        let operation = sanitize_operation(c.json(7, 0, "", &|| false).map_err(error)?)?;
        Ok((plugins, combat, operation, capabilities, identity))
    })();
    match read {
        Ok((plugins, combat, op, capabilities, identity)) => {
            result.capabilities = capabilities;
            result.connection_identity = Some(identity);
            result.connection = "connected";
            result.plugins = plugins;
            result.operation = Some(op);
            result.combat = combat.map(|s| Combat {
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
    validate_data_source_window(&window)?;
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
    validate_data_source_window(&window)?;
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
    let _ = value;
    let file = file.unwrap_or_default();
    let (command, argument) = match action.as_str() {
        "enable" | "disable" if valid_file(&file) => (
            match action.as_str() {
                "enable" => 4,
                "disable" => 5,
                _ => 6,
            },
            0,
        ),

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
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _reservation = state
            .reserve_plugin_control()
            .map_err(CommandError::from_core)?;
        if state.data_mode() != DataMode::Plugin {
            return Err(unsupported());
        }
        if command != 8
            && matches!(
                state.capture_phase(),
                nte_dps_tool::core::live_capture::LiveCapturePhase::Starting
                    | nte_dps_tool::core::live_capture::LiveCapturePhase::Running
                    | nte_dps_tool::core::live_capture::LiveCapturePhase::Stopping
            )
        {
            return Err(error(ToolkitError::Busy));
        }
        let (c, capabilities) = client()?;
        if !capabilities.contains(&command) {
            return Err(unsupported());
        }
        c.call(command, argument, &file, &|| false).map_err(error)?;
        Ok(())
    })
    .await
    .map_err(|_| error(ToolkitError::Failed))?
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ReleaseAction {
    HostStatus,
    Shutdown,
    CombatStatus,
    CombatReset,
    CombatExport,
    CombatStopExport,
    CombatOperation,
    CombatReport,
    EvidenceStatus,
    NetworkStatus,
    NetworkEnable,
    NetworkFlush,
    RuntimeRefresh,
    RadarRefresh,
    TraceEnable,
    TraceClear,
    HudStatus,
    HudConfigure,
    PrebattleSkillStatus,
    PrebattleSkillUnlock,
    FrameRateStatus,
    FrameRateConfigure,
    FrameRateRefresh,
    QuickUltimateStatus,
    QuickUltimateConfigure,
    TrainingStatus,
    TrainingWoodenEnable,
    TrainingWoodenDisable,
    TrainingOperation,
    UserStatus,
    UserRefresh,
    UserOperation,
    UserSnapshot,
    UserExport,
    UserCancel,
}
impl ReleaseAction {
    fn wire(self, value: Option<u32>) -> Result<(u32, u32), ToolkitError> {
        let command = match self {
            Self::HostStatus => 2,
            Self::Shutdown => 11,
            Self::CombatStatus => 100,
            Self::CombatReset => 103,
            Self::CombatExport => 104,
            Self::CombatStopExport => 105,
            Self::CombatOperation => 106,
            Self::CombatReport => 107,
            Self::EvidenceStatus => 108,
            Self::NetworkStatus => 109,
            Self::NetworkEnable => 110,
            Self::NetworkFlush => 113,
            Self::RuntimeRefresh => 114,
            Self::RadarRefresh => 115,
            Self::TraceEnable => 116,
            Self::TraceClear => 117,
            Self::HudStatus => 118,
            Self::HudConfigure => 119,
            Self::PrebattleSkillStatus => 120,
            Self::PrebattleSkillUnlock => 121,
            Self::FrameRateStatus => 122,
            Self::FrameRateConfigure => 123,
            Self::FrameRateRefresh => 124,
            Self::QuickUltimateStatus => 125,
            Self::QuickUltimateConfigure => 126,
            Self::TrainingStatus => 400,
            Self::TrainingWoodenEnable | Self::TrainingWoodenDisable => 403,
            Self::TrainingOperation => 410,
            Self::UserStatus => 300,
            Self::UserRefresh => 301,
            Self::UserOperation => 302,
            Self::UserSnapshot => 303,
            Self::UserExport => 304,
            Self::UserCancel => 305,
        };
        let argument = if matches!(
            self,
            Self::NetworkEnable
                | Self::TraceEnable
                | Self::FrameRateConfigure
                | Self::QuickUltimateConfigure
        ) {
            value
                .filter(|v| *v <= 1)
                .ok_or(ToolkitError::InvalidProtocol)?
        } else if matches!(self, Self::HudConfigure) {
            value
                .filter(|v| *v <= 31)
                .ok_or(ToolkitError::InvalidProtocol)?
        } else {
            if value.is_some() {
                return Err(ToolkitError::InvalidProtocol);
            }
            match self {
                Self::TrainingWoodenEnable => 9,
                Self::TrainingWoodenDisable => 10,
                _ => 0,
            }
        };
        Ok((command, argument))
    }
    fn requires_idle(self) -> bool {
        matches!(
            self,
            Self::CombatReset | Self::CombatStopExport | Self::Shutdown
        )
    }
    fn requires_confirmation(self) -> bool {
        matches!(
            self,
            Self::CombatReset
                | Self::CombatStopExport
                | Self::Shutdown
                | Self::TraceClear
                | Self::UserCancel
                | Self::PrebattleSkillUnlock
                | Self::TrainingWoodenEnable
                | Self::TrainingWoodenDisable
        )
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReleaseResult {
    command: u32,
    preview: String,
    total_bytes: usize,
    truncated: bool,
}
fn preview(command: u32, bytes: &[u8]) -> Result<ReleaseResult, ToolkitError> {
    if bytes.len() > nte_dps_tool::platform::toolkit::MAX_BLOB_BYTES {
        return Err(ToolkitError::TooLarge);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ToolkitError::InvalidProtocol)?;
    let mut end = text.len().min(8192);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Ok(ReleaseResult {
        command,
        preview: text[..end].to_owned(),
        total_bytes: bytes.len(),
        truncated: end < bytes.len(),
    })
}

#[tauri::command]
pub(crate) async fn release_plugin_action(
    action: ReleaseAction,
    value: Option<u32>,
    expected_identity: String,
    confirmed: bool,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<ReleaseResult, CommandError> {
    console::validate_window(&window)?;
    if state.data_mode() != DataMode::Plugin {
        return Err(unsupported());
    }
    if expected_identity.is_empty() || expected_identity.len() > 64 {
        return Err(error(ToolkitError::InvalidProtocol));
    }
    if action.requires_confirmation() && !confirmed {
        return Err(CommandError::mod_studio_risk_acknowledgement_required());
    }
    if action.requires_idle()
        && matches!(
            state.capture_phase(),
            nte_dps_tool::core::live_capture::LiveCapturePhase::Starting
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Running
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Stopping
        )
    {
        return Err(error(ToolkitError::Busy));
    }
    let (command, argument) = action.wire(value).map_err(error)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _reservation = state
            .reserve_plugin_control()
            .map_err(CommandError::from_core)?;
        if state.data_mode() != DataMode::Plugin {
            return Err(unsupported());
        }
        if action.requires_idle()
            && matches!(
                state.capture_phase(),
                nte_dps_tool::core::live_capture::LiveCapturePhase::Starting
                    | nte_dps_tool::core::live_capture::LiveCapturePhase::Running
                    | nte_dps_tool::core::live_capture::LiveCapturePhase::Stopping
            )
        {
            return Err(error(ToolkitError::Busy));
        }
        let (c, capabilities) = client()?;
        if c.identity() != expected_identity {
            return Err(error(ToolkitError::SessionChanged));
        }
        if !capabilities.contains(&command) {
            return Err(unsupported());
        }
        let bytes = c.call(command, argument, "", &|| false).map_err(error)?;
        preview(command, &bytes).map_err(error)
    })
    .await
    .map_err(|_| error(ToolkitError::Failed))?
}

#[tauri::command]
pub(crate) async fn set_host_loading_method(
    method: ModStudioLoadingMethod,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<PluginPanel, CommandError> {
    console::validate_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = state
            .reserve_plugin_control()
            .map_err(CommandError::from_core)?;
        state
            .set_mod_studio_loading_method(method)
            .map_err(|_| CommandError::settings_config_save_failed())?;
        Ok(snapshot(&state))
    })
    .await
    .map_err(|_| error(ToolkitError::Failed))?
}

fn directory_error() -> CommandError {
    CommandError {
        code: "plugin_game_directory_invalid",
        message_key: "Select a valid game directory containing HTGame.exe. No files were changed.",
        message_arguments: vec![],
        diagnostic_line: None,
    }
}

#[tauri::command]
pub(crate) async fn select_host_game_directory(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<PluginPanel, CommandError> {
    console::validate_window(&window)?;
    // No reservation or hot lock is held while the native picker is open.
    let selected = crate::file_dialog::choose_game_directory(&window)
        .await
        .map_err(|_| CommandError {
            code: "plugin_game_directory_dialog_failed",
            message_key: "Game directory selection failed.",
            message_arguments: vec![],
            diagnostic_line: None,
        })?;
    console::validate_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let crate::file_dialog::DialogOutcome::Selected(path) = selected {
            let directory =
                nte_dps_tool::platform::mods_plugin::resolve_manual_game_directory(&path)
                    .map_err(|_| directory_error())?;
            let text = directory
                .to_str()
                .filter(|s| {
                    s.len() <= nte_dps_tool::storage::config::MOD_STUDIO_GAME_DIRECTORY_MAX_BYTES
                        && !s.chars().any(|c| matches!(c, '\0' | '\r' | '\n'))
                })
                .ok_or_else(directory_error)?;
            let _permit = state
                .reserve_plugin_control()
                .map_err(CommandError::from_core)?;
            state
                .set_proxy_game_directory(Some(text.to_owned()))
                .map_err(|_| CommandError::settings_config_save_failed())?;
        }
        Ok(snapshot(&state))
    })
    .await
    .map_err(|_| error(ToolkitError::Failed))?
}

#[tauri::command]
pub(crate) async fn clear_host_game_directory(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<PluginPanel, CommandError> {
    console::validate_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = state
            .reserve_plugin_control()
            .map_err(CommandError::from_core)?;
        state
            .set_proxy_game_directory(None)
            .map_err(|_| CommandError::settings_config_save_failed())?;
        Ok(snapshot(&state))
    })
    .await
    .map_err(|_| error(ToolkitError::Failed))?
}

#[tauri::command]
pub(crate) async fn uninstall_proxy_host(
    state: State<'_, AppState>,
    window: WebviewWindow,
    confirmed: bool,
) -> Result<PluginPanel, CommandError> {
    console::validate_window(&window)?;
    if !confirmed {
        return Err(CommandError {
            code: "plugin_confirmation_required",
            message_key: "Confirm the plugin operation before continuing.",
            message_arguments: vec![],
            diagnostic_line: None,
        });
    }
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = state
            .reserve_plugin_control()
            .map_err(CommandError::from_core)?;
        if matches!(
            state.capture_phase(),
            nte_dps_tool::core::live_capture::LiveCapturePhase::Starting
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Running
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Stopping
        ) {
            return Err(error(ToolkitError::Busy));
        }
        if network::game_process_id()
            .map_err(|_| error(ToolkitError::Failed))?
            .is_some()
        {
            return Err(CommandError {
                code: "plugin_close_game_required",
                message_key: "Close the game before removing the proxy host.",
                message_arguments: vec![],
                diagnostic_line: None,
            });
        }
        let manual = state.proxy_game_directory();
        let destination = nte_dps_tool::platform::mods_plugin::toolkit_game_directory(
            manual.as_deref().map(std::path::Path::new),
        )
        .map_err(|_| directory_error())?;
        nte_dps_tool::core::mod_market::uninstall_proxy(
            &nte_dps_tool::storage::paths::toolkit_dir().join("game"),
            &destination,
        )
        .map_err(CommandError::from_mod_market)?;
        Ok(snapshot(&state))
    })
    .await
    .map_err(|_| error(ToolkitError::Failed))?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HostLaunchResult {
    panel: PluginPanel,
    outcome: &'static str,
}

#[tauri::command]
pub(crate) async fn launch_plugin_host(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<HostLaunchResult, CommandError> {
    console::validate_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = state
            .reserve_plugin_control()
            .map_err(CommandError::from_core)?;
        if state.data_mode() != DataMode::Plugin {
            return Err(CommandError::mod_studio_risk_acknowledgement_required());
        }
        if matches!(
            state.capture_phase(),
            nte_dps_tool::core::live_capture::LiveCapturePhase::Starting
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Running
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Stopping
        ) {
            return Err(error(ToolkitError::Busy));
        }
        let root = nte_dps_tool::storage::paths::toolkit_dir();
        let method = state.mod_studio_loading_method();
        let runtime = runtime_directory(&root, method)?;
        if method == ModStudioLoadingMethod::Proxy {
            if network::game_process_id().map_err(|_| error(ToolkitError::Failed))?.is_some() {
                return Err(CommandError { code: "plugin_close_game_required", message_key: "Close the game before deploying the proxy host, then restart it to load the host.", message_arguments: vec![], diagnostic_line: None });
            }
            let manual = state.proxy_game_directory();
            let destination = nte_dps_tool::platform::mods_plugin::toolkit_game_directory(manual.as_deref().map(std::path::Path::new))
                .map_err(|_| if manual.is_some() { directory_error() } else { CommandError { code: "plugin_game_directory_unavailable", message_key: "A unique game installation could not be confirmed. No files were changed.", message_arguments: vec![], diagnostic_line: None } })?;
            nte_dps_tool::core::mod_market::deploy_proxy(&runtime, &destination).map_err(CommandError::from_mod_market)?;
            return Ok(HostLaunchResult { panel: snapshot(&state), outcome: "proxyDeployed" });
        }
        let root = std::fs::canonicalize(root).map_err(|_| error(ToolkitError::Unavailable))?;
        let runtime = root.join("game");
        // Execute only the managed package's compiled Loader. It owns UAC, driver
        // cleanup and bounded handshake. Never kill it while cleanup may be pending.
        // Raw output can contain local paths; do not expose it to logs or the UI.
        let mut command = std::process::Command::new(root.join("tools/NTE-Loader.exe"));
        command
            .arg("--runtime-dir")
            .arg(runtime)
            .arg("--driver-path")
            .arg(root.join("driver/uetools.sys"))
            .arg("--json")
            .current_dir(&root)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW; UAC remains native.
        }
        let mut child = command.spawn().map_err(|e| loader_spawn_failed(&e))?;
        // Drain beyond the receipt budget so driver cleanup cannot block on stdout.
        // Never forward raw output or local paths.
        let receipt = child.stdout.take().map(read_loader_receipt);
        let status = child.wait().map_err(|e| loader_spawn_failed(&e))?;
        if !status.success() {
            let receipt = receipt.and_then(Result::ok).unwrap_or_default();
            return Err(loader_receipt_error(&receipt, status.code()));
        }
        // Exit 0 is not treated as a live connection: independently verify IPC.
        client()?;
        Ok(HostLaunchResult { panel: snapshot(&state), outcome: "connected" })
    })
    .await
    .map_err(|_| error(ToolkitError::Failed))?
}

const MAX_LOADER_RECEIPT_BYTES: usize = 64 * 1024;

fn read_loader_receipt(mut output: impl std::io::Read) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    output
        .by_ref()
        .take(MAX_LOADER_RECEIPT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    std::io::copy(&mut output, &mut std::io::sink())?;
    Ok(bytes)
}

fn loader_spawn_failed(error: &std::io::Error) -> CommandError {
    CommandError {
        code: "plugin_loader_launch_failed",
        message_key: "Could not start or wait for Loader (OS error {}). Check the managed Loader file and access permissions.",
        message_arguments: vec![
            error
                .raw_os_error()
                .map_or("unknown".into(), |n| n.to_string()),
        ],
        diagnostic_line: None,
    }
}

fn loader_receipt_error(bytes: &[u8], exit: Option<i32>) -> CommandError {
    let receipt = (bytes.len() <= MAX_LOADER_RECEIPT_BYTES)
        .then(|| serde_json::from_slice::<serde_json::Value>(bytes).ok())
        .flatten();
    let receipt = receipt.filter(|r| r["schemaVersion"] == 1 && r["ok"] == false);
    let mut phase = "unknown";
    let mut reason = "unclassified";
    let mut win32 = "unknown".to_owned();
    let mut cleanup = "unknown".to_owned();
    if let Some(r) = receipt {
        phase = match r["phase"].as_str() {
            Some("arguments") => "arguments",
            Some("elevation") => "elevation",
            Some("driver_inspect") => "driver_inspect",
            Some("target") => "target",
            Some("host_check") => "host_check",
            Some("driver_start") => "driver_start",
            Some("load") => "load",
            Some("connect") => "connect",
            Some("driver_stop") => "driver_stop",
            _ => "unknown",
        };
        // Exact allowlist of path-free errors from the current native Loader.
        // Unknown text remains private; a future Loader cannot inject paths here.
        reason = match r["error"].as_str() {
            Some("expected exactly one HTGame.exe; start the game or select --pid explicitly") => {
                "target_not_unique"
            }
            Some("target is not HTGame.exe") => "target_wrong_image",
            Some("target must be native Windows x64") => "target_wrong_architecture",
            Some(
                "target process has exited"
                | "target process exited"
                | "target exited before Toolkit IPC ready",
            ) => "target_exited",
            Some(
                "host must resolve to d3d12.dll, not a plugin"
                | "plugin image is not a host; only the host may load plugins",
            ) => "host_wrong_image",
            Some("driver lacks the required memory protocol marker") => "driver_protocol_mismatch",
            Some(
                "UETools has an orphan service/channel; normal SCM unload is unavailable; no forced unload",
            ) => "driver_orphan_channel",
            Some(
                "previous driver lifecycle was abandoned; cleanup only, do not reinject automatically",
            ) => "driver_abandoned_lifecycle",
            Some("D3D12 device creation failed; cannot install the reflective trigger") => {
                "d3d12_device_failed"
            }
            Some("D3D12 command queue creation failed") => "d3d12_queue_failed",
            Some("DXGI factory creation failed") => "dxgi_factory_failed",
            Some("DXGI swapchain creation failed; cannot install the reflective trigger") => {
                "dxgi_swapchain_failed"
            }
            Some("host library load returned null (DllMain or path failure)") => {
                "host_library_load_failed"
            }
            Some("Toolkit heartbeat stale") => "ipc_heartbeat_stale",
            Some("Toolkit response identity mismatch") => "ipc_identity_mismatch",
            Some("Toolkit Describe returned failure") => "ipc_describe_failed",
            _ => "unclassified",
        };
        let number = |v: &serde_json::Value| {
            v.as_u64()
                .filter(|n| *n <= u32::MAX as u64)
                .map(|n| n.to_string())
        };
        win32 = number(&r["win32"]).unwrap_or_else(|| "unknown".into());
        cleanup = if r["driver"]["stopped"] == true {
            "stopped".into()
        } else {
            number(&r["driver"]["cleanupWin32"]).unwrap_or_else(|| "unconfirmed".into())
        };
    }
    let code = match phase {
        "elevation" => "plugin_loader_elevation_failed",
        "driver_inspect" | "driver_start" | "driver_stop" => "plugin_loader_driver_failed",
        "target" => "plugin_loader_target_failed",
        "host_check" => "plugin_loader_host_invalid",
        "load" => "plugin_loader_load_failed",
        "connect" => "plugin_loader_handshake_failed",
        _ => "plugin_loader_result_invalid",
    };
    CommandError {
        code,
        message_key: "Loader failed: stage={}, reason={}, exit={}, Win32={}, driver cleanup={}. Check this stage before retrying; host startup may have been attempted.",
        message_arguments: vec![
            phase.into(),
            reason.into(),
            exit.map_or("unknown".into(), |n| n.to_string()),
            win32,
            cleanup,
        ],
        diagnostic_line: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loader_errors_keep_stage_and_os_codes_without_paths_or_raw_output() {
        let receipt = br#"{"schemaVersion":1,"ok":false,"phase":"driver_start","win32":577,"error":"private path","driver":{"stopped":true}}"#;
        let error = loader_receipt_error(receipt, Some(1));
        assert_eq!(error.code, "plugin_loader_driver_failed");
        assert_eq!(
            error.message_arguments,
            ["driver_start", "unclassified", "1", "577", "stopped"]
        );
        assert!(!format!("{:?}", error.message_arguments).contains("private"));
        let target = loader_receipt_error(br#"{"schemaVersion":1,"ok":false,"phase":"target","error":"expected exactly one HTGame.exe; start the game or select --pid explicitly"}"#, Some(1));
        assert_eq!(target.message_arguments[1], "target_not_unique");
        assert_eq!(
            loader_receipt_error(b"not json", Some(7)).code,
            "plugin_loader_result_invalid"
        );
        assert_eq!(
            loader_receipt_error(&vec![b' '; MAX_LOADER_RECEIPT_BYTES + 1], None).code,
            "plugin_loader_result_invalid"
        );
        let mut input = std::io::Cursor::new(vec![b'x'; MAX_LOADER_RECEIPT_BYTES * 2]);
        assert_eq!(
            read_loader_receipt(&mut input).unwrap().len(),
            MAX_LOADER_RECEIPT_BYTES + 1
        );
        assert_eq!(input.position(), (MAX_LOADER_RECEIPT_BYTES * 2) as u64);
    }
    #[test]
    fn loading_methods_share_the_managed_runtime_layout() {
        let root = std::env::temp_dir().join(format!("nte-managed-layout-{}", std::process::id()));
        std::fs::create_dir_all(root.join("game")).unwrap();
        std::fs::create_dir_all(root.join("tools")).unwrap();
        std::fs::create_dir_all(root.join("driver")).unwrap();
        assert!(runtime_directory(&root, ModStudioLoadingMethod::Proxy).is_err());
        std::fs::write(root.join("game/d3d12.dll"), []).unwrap();
        assert_eq!(
            runtime_directory(&root, ModStudioLoadingMethod::Proxy).unwrap(),
            root.join("game")
        );
        assert!(runtime_directory(&root, ModStudioLoadingMethod::Loader).is_err());
        std::fs::write(root.join("tools/NTE-Loader.exe"), []).unwrap();
        std::fs::write(root.join("driver/uetools.sys"), []).unwrap();
        assert_eq!(
            runtime_directory(&root, ModStudioLoadingMethod::Loader).unwrap(),
            root.join("game")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn release_controls_reject_developer_and_retired_commands_and_bad_arguments() {
        for name in [
            "mcp",
            "dumper7",
            "networkClear",
            "networkLogMode",
            "script",
            "equipmentWrite",
            "runtimeScan",
            "runtimeStatus",
        ] {
            assert!(serde_json::from_value::<ReleaseAction>(serde_json::json!(name)).is_err());
        }
        assert_eq!(ReleaseAction::UserRefresh.wire(None).unwrap(), (301, 0));
        assert!(ReleaseAction::UserRefresh.wire(Some(0)).is_err());
        assert!(ReleaseAction::NetworkEnable.wire(None).is_err());
        assert!(ReleaseAction::NetworkEnable.wire(Some(2)).is_err());
        assert_eq!(
            ReleaseAction::NetworkEnable.wire(Some(1)).unwrap(),
            (110, 1)
        );
        assert!(ReleaseAction::Shutdown.requires_confirmation());
        assert!(ReleaseAction::CombatReset.requires_idle());
        assert_eq!(ReleaseAction::HudStatus.wire(None).unwrap(), (118, 0));
        assert_eq!(
            ReleaseAction::PrebattleSkillStatus.wire(None).unwrap(),
            (120, 0)
        );
        assert_eq!(
            ReleaseAction::PrebattleSkillUnlock.wire(None).unwrap(),
            (121, 0)
        );
        assert!(ReleaseAction::PrebattleSkillUnlock.requires_confirmation());
        assert!(!ReleaseAction::PrebattleSkillStatus.requires_confirmation());
        assert!(ReleaseAction::PrebattleSkillUnlock.wire(Some(1)).is_err());

        assert_eq!(ReleaseAction::HudConfigure.wire(Some(0)).unwrap(), (119, 0));
        assert_eq!(
            ReleaseAction::HudConfigure.wire(Some(31)).unwrap(),
            (119, 31)
        );
        assert!(ReleaseAction::HudConfigure.wire(Some(32)).is_err());
        assert!(ReleaseAction::HudConfigure.wire(None).is_err());
        assert!(!ReleaseAction::HudConfigure.requires_idle());
    }
    #[test]
    fn current_release_wire_arguments_match_combat_and_wooden_only_training() {
        for (action, command) in [
            (ReleaseAction::FrameRateConfigure, 123),
            (ReleaseAction::QuickUltimateConfigure, 126),
        ] {
            assert_eq!(action.wire(Some(0)).unwrap(), (command, 0));
            assert_eq!(action.wire(Some(1)).unwrap(), (command, 1));
            assert!(action.wire(None).is_err());
            assert!(action.wire(Some(2)).is_err());
        }
        for (action, argument) in [
            (ReleaseAction::TrainingWoodenEnable, 9),
            (ReleaseAction::TrainingWoodenDisable, 10),
        ] {
            assert_eq!(action.wire(None).unwrap(), (403, argument));
            assert!(action.wire(Some(argument)).is_err());
            assert!(action.requires_confirmation());
        }
        assert_eq!(ReleaseAction::TrainingStatus.wire(None).unwrap(), (400, 0));
        assert_eq!(
            ReleaseAction::TrainingOperation.wire(None).unwrap(),
            (410, 0)
        );
        assert_eq!(
            ReleaseAction::FrameRateRefresh.wire(None).unwrap(),
            (124, 0)
        );
        assert_eq!(
            ReleaseAction::QuickUltimateStatus.wire(None).unwrap(),
            (125, 0)
        );
        for name in [
            "trainingEnter",
            "trainingSpawn",
            "trainingReset",
            "trainingClear",
            "performanceStatus",
        ] {
            assert!(serde_json::from_value::<ReleaseAction>(serde_json::json!(name)).is_err());
        }
    }
    #[test]
    fn release_preview_is_bounded_utf8_and_does_not_claim_full_content() {
        let data = "数".repeat(4000);
        let r = preview(303, data.as_bytes()).unwrap();
        assert!(r.truncated);
        assert_eq!(r.total_bytes, 12000);
        assert!(r.preview.len() <= 8192);
        assert!(preview(303, &[255]).is_err());
        assert!(!preview(300, b"{}").unwrap().truncated);
    }

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
