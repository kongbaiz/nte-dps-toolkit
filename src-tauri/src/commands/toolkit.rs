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
use std::path::PathBuf;
use tauri::{State, WebviewWindow};

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
        contract_version: 4,
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
            Self::UserStatus => 300,
            Self::UserRefresh => 301,
            Self::UserOperation => 302,
            Self::UserSnapshot => 303,
            Self::UserExport => 304,
            Self::UserCancel => 305,
        };
        let argument = if matches!(self, Self::NetworkEnable | Self::TraceEnable) {
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
            0
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
            let destination = nte_dps_tool::platform::mods_plugin::automatic_toolkit_game_directory()
                .map_err(|_| CommandError { code: "plugin_game_directory_unavailable", message_key: "A unique game installation could not be confirmed. No files were changed.", message_arguments: vec![], diagnostic_line: None })?;
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
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW; UAC remains native.
        }
        let status = command.status().map_err(|_| loader_failed())?;
        if !status.success() {
            return Err(loader_failed());
        }
        // Exit 0 is not treated as a live connection: independently verify IPC.
        client()?;
        Ok(HostLaunchResult { panel: snapshot(&state), outcome: "connected" })
    })
    .await
    .map_err(|_| error(ToolkitError::Failed))?
}

fn loader_failed() -> CommandError {
    CommandError {
        code: "plugin_loader_failed",
        message_key: "Loader did not confirm host startup. Check the Toolkit package, driver and UAC result before retrying.",
        message_arguments: vec![],
        diagnostic_line: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
