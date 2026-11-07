use crate::{
    contract::CommandError,
    state::AppState,
    windows::{console, main_dps},
};
use nte_dps_tool::{
    core::toolkit::{CombatStatus, DataMode},
    platform::{
        network,
        toolkit::{ToolkitClient, ToolkitDescription, ToolkitError},
    },
    storage::config::ModStudioLoadingMethod,
};
use serde::{Deserialize, Serialize};
use std::{io::Read, path::PathBuf};
use tauri::{State, WebviewWindow};
use ts_rs::TS;

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginPanel {
    connection_identity: Option<String>,
    collector_active: bool,
    capabilities: Vec<u32>,
    mode: DataMode,
    loading_method: ModStudioLoadingMethod,
    connection: PluginConnection,
    plugins: Vec<Plugin>,
    combat: Option<Combat>,
    operation: Option<PluginOperation>,
    components_ready: bool,
    game_directory: Option<String>,
}
#[derive(Clone, Copy, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
enum PluginConnection {
    NotRequested,
    Connected,
    Unavailable,
    Busy,
    Error,
}
#[derive(Clone, Copy, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
enum PluginState {
    Loaded,
    Unloaded,
    UnloadPending,
    Failed,
    Discovered,
    Faulted,
    Blocked,
    ReleasePending,
    Validated,
    FaultedUnloaded,
    Rejected,
}
#[derive(Serialize, Deserialize, TS)]
#[ts(export)]
pub(crate) struct Plugin {
    file: String,
    state: PluginState,
    #[serde(
        default,
        rename = "serviceOwner",
        deserialize_with = "owner_string",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(optional)]
    service_owner: Option<String>,
}
fn owner_string<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Ok(Option::<u64>::deserialize(deserializer)?.map(|owner| owner.to_string()))
}
#[derive(Serialize, TS)]
#[ts(export)]
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
#[derive(Serialize, Deserialize, TS)]
#[ts(export)]
pub(crate) struct PluginOperation {
    state: String,
    error: String,
}

fn sanitize_operation(mut operation: PluginOperation) -> Result<PluginOperation, CommandError> {
    if !matches!(
        operation.state.as_str(),
        "idle" | "queued" | "pending" | "completed" | "failed" | "rejected"
    ) || operation.error.len() > 512
    {
        return Err(CommandError::from(ToolkitError::InvalidProtocol));
    }
    // Native diagnostics can contain game-directory paths. Expose a stable key only.
    if !operation.error.is_empty() {
        operation.error = "The plugin operation failed. Query its status before retrying.".into();
    }
    Ok(operation)
}

pub(crate) fn unsupported() -> CommandError {
    ToolkitError::Unsupported.into()
}

impl From<ToolkitError> for CommandError {
    fn from(e: ToolkitError) -> Self {
        let (code, key) = match e {
            ToolkitError::Unavailable => (
                "plugin_unavailable",
                "UE Tools is not connected. Load the compiled Toolkit package in the game first.",
            ),
            ToolkitError::OperationUnavailable => (
                "plugin_operation_unavailable",
                "The host is connected, but this plugin operation is unavailable. Check plugin diagnostics and game readiness.",
            ),
            ToolkitError::Stopping => (
                "plugin_stopping",
                "The plugin host is stopping. Wait for shutdown to finish before reconnecting.",
            ),
            ToolkitError::Busy => (
                "plugin_busy",
                "The plugin is busy. Query its status before retrying.",
            ),
            ToolkitError::SessionChanged => (
                "plugin_session_changed",
                "The plugin session changed. Refresh before continuing.",
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
        CommandError::new(code, key)
    }
}
pub(super) fn client() -> Result<(ToolkitClient, ToolkitDescription), CommandError> {
    let pid = network::game_process_id()
        .map_err(|_| CommandError::from(ToolkitError::Failed))?
        .ok_or_else(|| CommandError::from(ToolkitError::Unavailable))?;
    let c = ToolkitClient::open(pid)?;
    let capabilities = c.describe()?;
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
        return Err(CommandError::from(ToolkitError::Unsupported));
    }
    if !runtime.join("d3d12.dll").is_file() || runtime.join("dwmapi.dll").exists() {
        return Err(CommandError::from(ToolkitError::Unsupported));
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
        connection: PluginConnection::NotRequested,
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
        let identity = capabilities.operation_session;
        let capabilities = capabilities.commands;
        let plugins = plugin_rows(c.json(3, 0, "", &|| false)?)?;
        let combat: Option<CombatStatus> = if capabilities.contains(&100) {
            match c.json(100, 0, "", &|| false) {
                Ok(value) => Some(value),
                Err(
                    ToolkitError::Unavailable
                    | ToolkitError::OperationUnavailable
                    | ToolkitError::Stopping
                    | ToolkitError::Unsupported,
                ) => None,
                Err(e) => return Err(CommandError::from(e)),
            }
        } else {
            None
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
            return Err(CommandError::from(ToolkitError::InvalidProtocol));
        }
        let operation = sanitize_operation(c.json(7, 0, "", &|| false)?)?;
        Ok((plugins, combat, operation, capabilities, identity))
    })();
    match read {
        Ok((plugins, combat, op, capabilities, identity)) => {
            result.capabilities = capabilities;
            result.connection_identity = Some(identity);
            result.connection = PluginConnection::Connected;
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
                "plugin_unavailable" => PluginConnection::Unavailable,
                "plugin_busy" => PluginConnection::Busy,
                _ => PluginConnection::Error,
            }
        }
    }
    result
}
// A resident image and a replacement candidate can share a filename in PluginList.
fn plugin_rows(mut rows: Vec<Plugin>) -> Result<Vec<Plugin>, ToolkitError> {
    if rows.len() > 64 || rows.iter().any(|p| !valid_file(&p.file)) {
        return Err(ToolkitError::InvalidProtocol);
    }
    rows.sort_by_cached_key(|p| {
        (
            p.file.to_ascii_lowercase(),
            match p.state {
                PluginState::Blocked | PluginState::ReleasePending | PluginState::UnloadPending => {
                    0
                }
                PluginState::Loaded | PluginState::Faulted => 1,
                PluginState::Validated => 2,
                PluginState::Rejected | PluginState::Failed | PluginState::FaultedUnloaded => 3,
                PluginState::Unloaded | PluginState::Discovered => 4,
            },
        )
    });
    rows.dedup_by(|a, b| a.file.eq_ignore_ascii_case(&b.file));
    Ok(rows)
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
        .map_err(|_| CommandError::from(ToolkitError::Failed))
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
        state.set_data_mode(mode)?;
        // Choosing a mode does not discard a recorded session. The normal Start
        // command retains its existing replace-current confirmation contract.
        Ok(snapshot(&state))
    })
    .await
    .map_err(|_| CommandError::from(ToolkitError::Failed))?
}
#[derive(Clone, Copy, Debug, serde::Deserialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PluginControlAction {
    Enable,
    Disable,
}

#[tauri::command]
pub(crate) async fn control_plugin(
    action: PluginControlAction,
    file: String,
    expected_identity: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    console::validate_window(&window)?;
    if state.data_mode() != DataMode::Plugin {
        return Err(CommandError::mod_studio_risk_acknowledgement_required());
    }
    if !valid_file(&file) {
        return Err(CommandError::from(ToolkitError::Unsupported));
    }
    let command = match action {
        PluginControlAction::Enable => 4,
        PluginControlAction::Disable => 5,
    };
    // Combat collector owns start/stop. Never unload it under an active stream.
    if matches!(
        state.capture_phase(),
        nte_dps_tool::core::live_capture::LiveCapturePhase::Starting
            | nte_dps_tool::core::live_capture::LiveCapturePhase::Running
            | nte_dps_tool::core::live_capture::LiveCapturePhase::Stopping
    ) {
        return Err(CommandError::from(ToolkitError::Busy));
    }
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _reservation = state.reserve_plugin_control()?;
        if state.data_mode() != DataMode::Plugin {
            return Err(unsupported());
        }
        if matches!(
            state.capture_phase(),
            nte_dps_tool::core::live_capture::LiveCapturePhase::Starting
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Running
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Stopping
        ) {
            return Err(CommandError::from(ToolkitError::Busy));
        }
        let (c, capabilities) = client()?;
        if !capabilities.commands.contains(&command) {
            return Err(unsupported());
        }
        if expected_identity != capabilities.operation_session {
            return Err(ToolkitError::SessionChanged.into());
        }
        c.call(command, 0, &format!("{expected_identity}/{file}"), &|| {
            false
        })?;
        Ok(())
    })
    .await
    .map_err(|_| CommandError::from(ToolkitError::Failed))?
}
#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) enum HostAction {
    HostStatus,
    Shutdown,
}
impl HostAction {
    fn command(self) -> u32 {
        match self {
            Self::HostStatus => 2,
            Self::Shutdown => 11,
        }
    }
}
#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReleaseResult {
    command: u32,
    preview: String,
    total_bytes: usize,
    truncated: bool,
}
pub(super) fn preview(command: u32, bytes: &[u8]) -> Result<ReleaseResult, ToolkitError> {
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
pub(crate) async fn host_action(
    action: HostAction,
    expected_identity: String,
    confirmed: bool,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<ReleaseResult, CommandError> {
    console::validate_window(&window)?;
    if state.data_mode() != DataMode::Plugin {
        return Err(unsupported());
    }
    if matches!(action, HostAction::Shutdown) && !confirmed {
        return Err(CommandError::mod_studio_risk_acknowledgement_required());
    }
    if matches!(action, HostAction::Shutdown)
        && matches!(
            state.capture_phase(),
            nte_dps_tool::core::live_capture::LiveCapturePhase::Starting
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Running
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Stopping
        )
    {
        return Err(CommandError::from(ToolkitError::Busy));
    }
    let command = action.command();
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _reservation = state.reserve_plugin_control()?;
        if state.data_mode() != DataMode::Plugin {
            return Err(unsupported());
        }
        if matches!(action, HostAction::Shutdown)
            && matches!(
                state.capture_phase(),
                nte_dps_tool::core::live_capture::LiveCapturePhase::Starting
                    | nte_dps_tool::core::live_capture::LiveCapturePhase::Running
                    | nte_dps_tool::core::live_capture::LiveCapturePhase::Stopping
            )
        {
            return Err(CommandError::from(ToolkitError::Busy));
        }
        let (c, capabilities) = client()?;
        if capabilities.operation_session != expected_identity {
            return Err(CommandError::from(ToolkitError::SessionChanged));
        }
        if !capabilities.commands.contains(&command) {
            return Err(unsupported());
        }
        let bytes = c.call(command, 0, "", &|| false)?;
        preview(command, &bytes).map_err(CommandError::from)
    })
    .await
    .map_err(|_| CommandError::from(ToolkitError::Failed))?
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
        let _permit = state.reserve_plugin_control()?;
        state
            .set_mod_studio_loading_method(method)
            .map_err(|_| CommandError::settings_config_save_failed())?;
        Ok(snapshot(&state))
    })
    .await
    .map_err(|_| CommandError::from(ToolkitError::Failed))?
}

fn directory_error() -> CommandError {
    CommandError::new(
        "plugin_game_directory_invalid",
        "Select a valid game directory containing HTGame.exe. No files were changed.",
    )
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
            let _permit = state.reserve_plugin_control()?;
            state
                .set_proxy_game_directory(Some(text.to_owned()))
                .map_err(|_| CommandError::settings_config_save_failed())?;
        }
        Ok(snapshot(&state))
    })
    .await
    .map_err(|_| CommandError::from(ToolkitError::Failed))?
}

#[tauri::command]
pub(crate) async fn clear_host_game_directory(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<PluginPanel, CommandError> {
    console::validate_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = state.reserve_plugin_control()?;
        state
            .set_proxy_game_directory(None)
            .map_err(|_| CommandError::settings_config_save_failed())?;
        Ok(snapshot(&state))
    })
    .await
    .map_err(|_| CommandError::from(ToolkitError::Failed))?
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
        let _permit = state.reserve_plugin_control()?;
        if matches!(
            state.capture_phase(),
            nte_dps_tool::core::live_capture::LiveCapturePhase::Starting
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Running
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Stopping
        ) {
            return Err(CommandError::from(ToolkitError::Busy));
        }
        if network::game_process_id()
            .map_err(|_| CommandError::from(ToolkitError::Failed))?
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
    .map_err(|_| CommandError::from(ToolkitError::Failed))?
}

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HostLaunchResult {
    panel: PluginPanel,
    outcome: &'static str,
}

#[tauri::command]
pub(crate) async fn launch_plugin_host(
    state: State<'_, AppState>,
    window: WebviewWindow,
    force_overwrite: bool,
) -> Result<HostLaunchResult, CommandError> {
    console::validate_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = state.reserve_plugin_control()?;
        if state.data_mode() != DataMode::Plugin {
            return Err(CommandError::mod_studio_risk_acknowledgement_required());
        }
        if matches!(
            state.capture_phase(),
            nte_dps_tool::core::live_capture::LiveCapturePhase::Starting
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Running
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Stopping
        ) {
            return Err(CommandError::from(ToolkitError::Busy));
        }
        let root = nte_dps_tool::storage::paths::toolkit_dir();
        let method = state.mod_studio_loading_method();
        if force_overwrite && method != ModStudioLoadingMethod::Proxy {
            return Err(CommandError::from(ToolkitError::Busy));
        }
        let runtime = runtime_directory(&root, method)?;
        if method == ModStudioLoadingMethod::Proxy {
            deploy_proxy_host(&state, &runtime, force_overwrite)?;
            return Ok(HostLaunchResult {
                panel: snapshot(&state),
                outcome: "proxyDeployed",
            });
        }
        let root = std::fs::canonicalize(root)
            .map_err(|_| CommandError::from(ToolkitError::Unavailable))?;
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
        Ok(HostLaunchResult {
            panel: snapshot(&state),
            outcome: "connected",
        })
    })
    .await
    .map_err(|_| CommandError::from(ToolkitError::Failed))?
}

fn deploy_proxy_host(
    state: &AppState,
    runtime: &std::path::Path,
    force_overwrite: bool,
) -> Result<(), CommandError> {
    if network::game_process_id()
        .map_err(|_| CommandError::from(ToolkitError::Failed))?
        .is_some()
    {
        return Err(CommandError::new(
            "plugin_close_game_required",
            "Close the game before deploying the proxy host, then restart it to load the host.",
        ));
    }
    let manual = state.proxy_game_directory();
    let destination = nte_dps_tool::platform::mods_plugin::toolkit_game_directory(
        manual.as_deref().map(std::path::Path::new),
    )
    .map_err(|_| {
        if manual.is_some() {
            directory_error()
        } else {
            CommandError::new(
                "plugin_game_directory_unavailable",
                "A unique game installation could not be confirmed. No files were changed.",
            )
        }
    })?;
    nte_dps_tool::core::mod_market::deploy_proxy(runtime, &destination, force_overwrite)
        .map_err(CommandError::from_mod_market)
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
    CommandError::with_arguments(
        "plugin_loader_launch_failed",
        "Could not start or wait for Loader (OS error {}). Check the managed Loader file and access permissions.",
        vec![
            error
                .raw_os_error()
                .map_or("unknown".into(), |n| n.to_string()),
        ],
    )
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
    CommandError::with_arguments(
        code,
        "Loader failed: stage={}, reason={}, exit={}, Win32={}, driver cleanup={}. Check this stage before retrying; host startup may have been attempted.",
        vec![
            phase.into(),
            reason.into(),
            exit.map_or("unknown".into(), |n| n.to_string()),
            win32,
            cleanup,
        ],
    )
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
    fn remote_rejections_do_not_tell_users_to_reload_a_connected_host() {
        for (error, code) in [
            (
                ToolkitError::OperationUnavailable,
                "plugin_operation_unavailable",
            ),
            (ToolkitError::Stopping, "plugin_stopping"),
            (ToolkitError::Unavailable, "plugin_unavailable"),
        ] {
            assert_eq!(CommandError::from(error).code, code);
        }
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
    fn rejected_candidate_does_not_hide_resident_plugin() {
        let rows = serde_json::from_str(r#"[{"file":"Combat.dll","state":"rejected"},{"file":"Combat.dll","state":"loaded"},{"file":"Training.dll","state":"discovered"}]"#).unwrap();
        let rows = plugin_rows(rows).unwrap();
        assert!(matches!(
            rows.as_slice(),
            [
                Plugin {
                    state: PluginState::Loaded,
                    ..
                },
                Plugin {
                    state: PluginState::Discovered,
                    ..
                }
            ]
        ));
        assert!(
            sanitize_operation(PluginOperation {
                state: "idle".into(),
                error: String::new()
            })
            .is_ok()
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
