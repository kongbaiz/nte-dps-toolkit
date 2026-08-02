use serde::Serialize;

use nte_dps_tool::core::{
    CoreError, CoreErrorCode,
    hud::HudSnapshot,
    live_capture::{LiveCaptureIssue, LiveCapturePhase, LiveCaptureStatus},
};

pub(crate) mod abyss_values;
pub(crate) mod character_data;
pub(crate) mod diagnostics;
pub(crate) mod empty_curtain;
pub(crate) mod encrypted_ini;
pub(crate) mod history;
pub(crate) mod main_dps;
pub(crate) mod mod_studio;
pub(crate) mod packets;
pub(crate) mod resources;
pub(crate) mod settings;
pub(crate) mod skills;
pub(crate) mod timeline;

pub(crate) const TECHNICAL_CONTRACT_VERSION: u32 = 4;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TechnicalSnapshot {
    pub contract_version: u32,
    pub sequence: String,
    pub bridge_status: &'static str,
    pub adapter_version: &'static str,
    pub window_label: &'static str,
    pub uptime_ms: String,
    pub stream_interval_ms: u32,
    pub supported_locales: Vec<&'static str>,
    pub window: HudWindowSnapshot,
    pub capture: CaptureSnapshot,
    pub hud: HudSnapshot,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HudWindowSnapshot {
    pub passthrough: bool,
    pub always_on_top: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureSnapshot {
    pub phase: &'static str,
    pub message_key: &'static str,
    pub message_arguments: Vec<String>,
    pub issue: Option<CaptureIssueSnapshot>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureIssueSnapshot {
    pub code: &'static str,
    pub message_key: &'static str,
    pub message_arguments: Vec<String>,
}

impl From<LiveCaptureStatus> for CaptureSnapshot {
    fn from(status: LiveCaptureStatus) -> Self {
        let (phase, message_key) = match status.phase {
            LiveCapturePhase::Idle => ("idle", "No live capture task right now"),
            LiveCapturePhase::Starting => ("starting", "Starting live capture..."),
            LiveCapturePhase::Running => ("running", "A live capture task is running"),
            LiveCapturePhase::Stopping => ("stopping", "Stopping live capture..."),
            LiveCapturePhase::Stopped => ("stopped", "Stopped"),
            LiveCapturePhase::Failed => ("failed", "Capture parser stopped unexpectedly"),
        };
        Self {
            phase,
            message_key,
            message_arguments: Vec::new(),
            issue: status.issue.map(capture_issue_snapshot),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", content = "payload", rename_all = "camelCase")]
pub(crate) enum TechnicalEvent {
    Snapshot(TechnicalSnapshot),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubscriptionReceipt {
    pub subscription_id: String,
    pub stream_interval_ms: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommandError {
    pub code: &'static str,
    pub message_key: &'static str,
    pub message_arguments: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic_line: Option<u32>,
}

impl CommandError {
    pub(crate) fn main_dps(code: &'static str, message_key: &'static str) -> Self {
        Self {
            code,
            message_key,
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn history(code: &'static str, message_key: &'static str) -> Self {
        Self {
            code,
            message_key,
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn timeline(code: &'static str, message_key: &'static str) -> Self {
        Self {
            code,
            message_key,
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn skills(code: &'static str, message_key: &'static str) -> Self {
        Self {
            code,
            message_key,
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn empty_curtain(
        code: &'static str,
        message_key: &'static str,
        message_arguments: Vec<String>,
    ) -> Self {
        Self {
            code,
            message_key,
            message_arguments,
            diagnostic_line: None,
        }
    }

    pub(crate) fn character_data(
        code: &'static str,
        message_key: &'static str,
        message_arguments: Vec<String>,
    ) -> Self {
        Self {
            code,
            message_key,
            message_arguments,
            diagnostic_line: None,
        }
    }

    pub(crate) fn encrypted_ini(
        code: &'static str,
        message_key: &'static str,
        message_arguments: Vec<String>,
    ) -> Self {
        Self {
            code,
            message_key,
            message_arguments,
            diagnostic_line: None,
        }
    }

    pub(crate) fn resources(code: &'static str, message_key: &'static str) -> Self {
        Self {
            code,
            message_key,
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn diagnostics(code: &'static str, message_key: &'static str) -> Self {
        Self {
            code,
            message_key,
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn invalid_subscription_id() -> Self {
        Self {
            code: "invalid_subscription_id",
            message_key: "Technical subscription identifier is invalid.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn invalid_window() -> Self {
        Self {
            code: "invalid_window",
            message_key: "Technical command is not available for this window.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn window_operation_failed() -> Self {
        Self {
            code: "window_operation_failed",
            message_key: "HUD window operation failed.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn passthrough_hotkey_unavailable() -> Self {
        Self {
            code: "passthrough_hotkey_unavailable",
            message_key: "Global hotkeys are not ready; mouse passthrough was not enabled",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn invalid_hud_module() -> Self {
        Self {
            code: "invalid_hud_module",
            message_key: "HUD module is invalid.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn hud_config_save_failed() -> Self {
        Self {
            code: "hud_config_save_failed",
            message_key: "Failed to save HUD configuration.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn invalid_hud_option() -> Self {
        Self {
            code: "invalid_hud_option",
            message_key: "HUD option is invalid.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn invalid_hud_preset() -> Self {
        Self {
            code: "invalid_hud_preset",
            message_key: "HUD preset is invalid.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn invalid_settings_input() -> Self {
        Self {
            code: "invalid_settings_input",
            message_key: "Settings input is invalid.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn settings_config_save_failed() -> Self {
        Self {
            code: "settings_config_save_failed",
            message_key: "Failed to save settings.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn update_operation_busy() -> Self {
        Self {
            code: "update_operation_busy",
            message_key: "Another update operation is already in progress.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn update_component_unavailable() -> Self {
        Self {
            code: "update_component_unavailable",
            message_key: "The selected update component is no longer available.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn update_not_prepared() -> Self {
        Self {
            code: "update_not_prepared",
            message_key: "Download and verify the update before installing it.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn update_install_blocked(message_key: &'static str) -> Self {
        Self {
            code: "update_install_blocked",
            message_key,
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn team_data_invalid() -> Self {
        Self {
            code: "team_data_invalid",
            message_key: "Team DPS data is invalid.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn team_data_unavailable() -> Self {
        Self {
            code: "team_data_unavailable",
            message_key: "No team DPS data is available to export.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn team_data_export_failed() -> Self {
        Self {
            code: "team_data_export_failed",
            message_key: "Failed to export team data.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn abyss_values_unavailable() -> Self {
        Self {
            code: "abyss_values_unavailable",
            message_key: "Abyss monster values could not be loaded.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn abyss_team_unavailable() -> Self {
        Self {
            code: "abyss_team_unavailable",
            message_key: "The DPS data file has no team usable for this line",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn from_core(error: CoreError) -> Self {
        let issue = start_issue_snapshot(error.code);
        Self {
            code: issue.code,
            message_key: issue.message_key,
            message_arguments: issue.message_arguments,
            diagnostic_line: None,
        }
    }
}

fn capture_issue_snapshot(issue: LiveCaptureIssue) -> CaptureIssueSnapshot {
    match issue {
        LiveCaptureIssue::Start(code) => start_issue_snapshot(code),
        LiveCaptureIssue::RuntimeWarning => CaptureIssueSnapshot {
            code: "capture_warning",
            message_key: "Capture warning",
            message_arguments: Vec::new(),
        },
        LiveCaptureIssue::RuntimeError => CaptureIssueSnapshot {
            code: "capture_failed",
            message_key: "Capture parser stopped unexpectedly",
            message_arguments: Vec::new(),
        },
    }
}

fn start_issue_snapshot(code: CoreErrorCode) -> CaptureIssueSnapshot {
    let (code, message_key) = match code {
        CoreErrorCode::NpcapNotFound => (
            "npcap_not_found",
            "No usable capture device; confirm Npcap is installed",
        ),
        CoreErrorCode::GameProcessNotFound => ("game_not_detected", "Game not detected"),
        CoreErrorCode::CaptureDeviceNotFound => (
            "capture_device_not_found",
            "No matching capture device, cannot locate the game connection",
        ),
        CoreErrorCode::SystemProbeFailed => (
            "capture_environment_unavailable",
            "Capture environment unavailable",
        ),
        CoreErrorCode::CaptureAlreadyRunning => {
            ("capture_already_running", "Live capture is already running")
        }
        CoreErrorCode::CaptureNotRunning => ("capture_not_running", "Live capture is not running"),
    };
    CaptureIssueSnapshot {
        code,
        message_key,
        message_arguments: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    use nte_dps_tool::{
        core::hud::{HudProjectionOptions, project_hud},
        engine::model::CombatState,
        storage::config::HudConfig,
    };

    fn test_hud_snapshot() -> HudSnapshot {
        project_hud(
            &CombatState::default(),
            &HudConfig::detailed(),
            &HashSet::new(),
            HudProjectionOptions {
                preview_when_empty: true,
                ..HudProjectionOptions::default()
            },
        )
    }

    #[test]
    fn technical_snapshot_uses_camel_case_and_string_sequence() {
        let snapshot = TechnicalSnapshot {
            contract_version: TECHNICAL_CONTRACT_VERSION,
            sequence: "9007199254740992".to_owned(),
            bridge_status: "ready",
            adapter_version: "0.3.6",
            window_label: "hud-spike",
            uptime_ms: "12".to_owned(),
            stream_interval_ms: 100,
            supported_locales: vec!["en", "zh-CN"],
            window: HudWindowSnapshot {
                passthrough: false,
                always_on_top: true,
            },
            capture: LiveCaptureStatus::default().into(),
            hud: test_hud_snapshot(),
        };

        let value = serde_json::to_value(snapshot).expect("snapshot must serialize");

        assert_eq!(value["contractVersion"], TECHNICAL_CONTRACT_VERSION);
        assert_eq!(value["sequence"], "9007199254740992");
        assert_eq!(value["window"]["alwaysOnTop"], true);
        assert_eq!(value["capture"]["phase"], "idle");
        assert_eq!(value["hud"]["dataState"], "preview");
        assert_eq!(value["hud"]["version"], 2);
        assert_eq!(value["hud"]["characters"][0]["hits"], "38");
        assert_eq!(value["hud"]["timeline"]["buckets"][0]["hits"], "1");
        assert!(value.get("contract_version").is_none());
    }

    #[test]
    fn technical_event_has_stable_discriminant() {
        let event = TechnicalEvent::Snapshot(TechnicalSnapshot {
            contract_version: TECHNICAL_CONTRACT_VERSION,
            sequence: "1".to_owned(),
            bridge_status: "ready",
            adapter_version: "0.3.6",
            window_label: "hud-spike",
            uptime_ms: "12".to_owned(),
            stream_interval_ms: 100,
            supported_locales: vec!["en"],
            window: HudWindowSnapshot {
                passthrough: false,
                always_on_top: true,
            },
            capture: LiveCaptureStatus::default().into(),
            hud: test_hud_snapshot(),
        });

        let value = serde_json::to_value(event).expect("event must serialize");

        assert_eq!(value["event"], "snapshot");
        assert_eq!(
            value["payload"]["contractVersion"],
            TECHNICAL_CONTRACT_VERSION
        );
    }

    #[test]
    fn capture_status_maps_private_runtime_failures_to_stable_display_contract() {
        let capture: CaptureSnapshot = LiveCaptureStatus {
            phase: LiveCapturePhase::Failed,
            issue: Some(LiveCaptureIssue::RuntimeError),
        }
        .into();

        assert_eq!(capture.phase, "failed");
        let issue = capture.issue.expect("capture issue");
        assert_eq!(issue.code, "capture_failed");
        assert_eq!(issue.message_key, "Capture parser stopped unexpectedly");
    }
}
