use std::path::Path;

use nte_dps_tool::{
    core::{
        CoreError,
        diagnostics::{
            DiagnosticCheck, DiagnosticEnvironment, DiagnosticMessage, DiagnosticReport,
        },
        live_capture::LiveCapturePhase,
    },
    engine::{capture::RawCaptureSnapshot, model::CaptureQualitySummary},
};
use serde::Serialize;

use crate::state::AppState;

pub(crate) const DIAGNOSTICS_CONTRACT_VERSION: u32 = 3;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticsSnapshot {
    pub contract_version: u32,
    pub capture_generation: String,
    pub quality_generation: String,
    pub report_generation: String,
    pub adapter_version: &'static str,
    pub capture: DiagnosticsCaptureSnapshot,
    pub environment: Option<DiagnosticsEnvironmentSnapshot>,
    pub report: Option<DiagnosticsReportSnapshot>,
    pub quality: DiagnosticsQualitySnapshot,
    pub actions: DiagnosticsActionsSnapshot,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticsCaptureSnapshot {
    pub phase: &'static str,
    pub replay_running: bool,
    pub active_filter: Option<String>,
    pub dropped_history_archives: String,
    pub raw_capture: Option<DiagnosticsRawCaptureSnapshot>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticsRawCaptureSnapshot {
    pub file_name: Option<String>,
    pub packet_count: String,
    pub captured_bytes: String,
    pub write_error: bool,
    pub writing: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticsEnvironmentSnapshot {
    pub device_label: Option<String>,
    pub manual_device: bool,
    pub local_ip: Option<String>,
    pub game_connection: Option<DiagnosticsGameConnectionSnapshot>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticsGameConnectionSnapshot {
    pub pid: u32,
    pub local_ip: String,
    pub remote_ip: String,
    pub remote_port: u16,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticsReportSnapshot {
    pub failed_count: usize,
    pub warning_count: usize,
    pub checks: Vec<DiagnosticsCheckSnapshot>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticsCheckSnapshot {
    pub status: &'static str,
    pub title_key: &'static str,
    pub detail: DiagnosticsMessageSnapshot,
    pub suggestion: DiagnosticsMessageSnapshot,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticsMessageSnapshot {
    pub message_key: &'static str,
    pub message_arguments: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticsQualitySnapshot {
    pub source: &'static str,
    pub packet_count: usize,
    pub packets_with_hits: usize,
    pub hit_count: usize,
    pub outgoing_hits: String,
    pub outgoing_damage: f64,
    pub unknown_direction_hits: String,
    pub unknown_direction_damage: f64,
    pub incoming_hits: String,
    pub incoming_damage: f64,
    pub unknown_character_count: usize,
    pub unknown_character_hits: String,
    pub unmapped_skill_rows: usize,
    pub unmapped_skill_hits: String,
    pub unmapped_gameplay_effect_count: usize,
    pub time_stop_event_count: String,
    pub time_stop_interval_count: usize,
    pub abyss_event_count: String,
    pub server_damage_corrections: String,
    pub unattributed_server_damage_events: String,
    pub unattributed_server_damage: f64,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticsActionsSnapshot {
    pub can_import: bool,
    pub can_export_parsed: bool,
    pub can_export_raw: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticsActionResult {
    pub performed: bool,
    pub snapshot: DiagnosticsSnapshot,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", content = "payload", rename_all = "camelCase")]
pub(crate) enum DiagnosticsEvent {
    Snapshot(DiagnosticsSnapshot),
}

impl DiagnosticsSnapshot {
    pub(crate) fn from_state(state: &AppState) -> Result<Self, CoreError> {
        let (capture_generation, quality_generation, _) = state.diagnostics_revision()?;
        let (report_generation, diagnostic_run) = state.diagnostics_report_snapshot();
        let capture_input = state.diagnostics_input()?;
        let phase = state.capture_phase();
        let raw_capture = state.diagnostics_raw_capture()?;
        let inactive = matches!(
            phase,
            LiveCapturePhase::Idle | LiveCapturePhase::Stopped | LiveCapturePhase::Failed
        ) && !capture_input.replay_running;
        let can_export_raw = inactive
            && raw_capture.as_ref().is_some_and(|raw| {
                raw.packet_count > 0 && !raw.write_error && !raw.writing && raw.path.is_some()
            });
        let (environment, report) = diagnostic_run.map_or((None, None), |run| {
            (
                Some(DiagnosticsEnvironmentSnapshot::from(run.environment)),
                Some(DiagnosticsReportSnapshot::from(run.report)),
            )
        });
        Ok(Self {
            contract_version: DIAGNOSTICS_CONTRACT_VERSION,
            capture_generation: capture_generation.to_string(),
            quality_generation: quality_generation.to_string(),
            report_generation: report_generation.to_string(),
            adapter_version: env!("CARGO_PKG_VERSION"),
            capture: DiagnosticsCaptureSnapshot {
                phase: capture_phase_code(phase),
                replay_running: capture_input.replay_running,
                active_filter: capture_input.active_capture_filter,
                dropped_history_archives: capture_input.dropped_history_archives.to_string(),
                raw_capture: raw_capture.map(DiagnosticsRawCaptureSnapshot::from),
            },
            environment,
            report,
            quality: DiagnosticsQualitySnapshot::from(state.diagnostics_quality()?),
            actions: DiagnosticsActionsSnapshot {
                can_import: inactive,
                can_export_parsed: inactive && state.diagnostics_has_exportable_state()?,
                can_export_raw,
            },
        })
    }
}

impl From<RawCaptureSnapshot> for DiagnosticsRawCaptureSnapshot {
    fn from(raw: RawCaptureSnapshot) -> Self {
        Self {
            file_name: raw.path.as_deref().and_then(file_name),
            packet_count: raw.packet_count.to_string(),
            captured_bytes: raw.captured_bytes.to_string(),
            write_error: raw.write_error,
            writing: raw.writing,
        }
    }
}

impl From<DiagnosticEnvironment> for DiagnosticsEnvironmentSnapshot {
    fn from(environment: DiagnosticEnvironment) -> Self {
        Self {
            device_label: environment.device_label,
            manual_device: environment.manual_device,
            local_ip: environment.local_ip,
            game_connection: environment.game_connection.map(|network| {
                DiagnosticsGameConnectionSnapshot {
                    pid: network.pid,
                    local_ip: network.local_ip,
                    remote_ip: network.remote_ip,
                    remote_port: network.remote_port,
                }
            }),
        }
    }
}

impl From<DiagnosticReport> for DiagnosticsReportSnapshot {
    fn from(report: DiagnosticReport) -> Self {
        Self {
            failed_count: report.failed_count(),
            warning_count: report.warning_count(),
            checks: report
                .checks
                .into_iter()
                .map(DiagnosticsCheckSnapshot::from)
                .collect(),
        }
    }
}

impl From<DiagnosticCheck> for DiagnosticsCheckSnapshot {
    fn from(check: DiagnosticCheck) -> Self {
        Self {
            status: check.status.code(),
            title_key: check.title_key,
            detail: DiagnosticsMessageSnapshot::from(check.detail),
            suggestion: DiagnosticsMessageSnapshot::from(check.suggestion),
        }
    }
}

impl From<DiagnosticMessage> for DiagnosticsMessageSnapshot {
    fn from(message: DiagnosticMessage) -> Self {
        Self {
            message_key: message.key,
            message_arguments: message.arguments,
        }
    }
}

impl From<CaptureQualitySummary> for DiagnosticsQualitySnapshot {
    fn from(quality: CaptureQualitySummary) -> Self {
        Self {
            source: match quality.source {
                nte_dps_tool::engine::model::CaptureQualitySource::Live => "live",
                nte_dps_tool::engine::model::CaptureQualitySource::PcapngReplay => "pcapng_replay",
                nte_dps_tool::engine::model::CaptureQualitySource::JsonReplay => "json_replay",
                nte_dps_tool::engine::model::CaptureQualitySource::Unknown => "unknown",
            },
            packet_count: quality.packet_count,
            packets_with_hits: quality.packets_with_hits,
            hit_count: quality.hit_count,
            outgoing_hits: quality.outgoing_hits.to_string(),
            outgoing_damage: quality.outgoing_damage,
            unknown_direction_hits: quality.unknown_direction_hits.to_string(),
            unknown_direction_damage: quality.unknown_direction_damage,
            incoming_hits: quality.incoming_hits.to_string(),
            incoming_damage: quality.incoming_damage,
            unknown_character_count: quality.unknown_character_count,
            unknown_character_hits: quality.unknown_character_hits.to_string(),
            unmapped_skill_rows: quality.unmapped_skill_rows,
            unmapped_skill_hits: quality.unmapped_skill_hits.to_string(),
            unmapped_gameplay_effect_count: quality.unmapped_gameplay_effect_count,
            time_stop_event_count: quality.time_stop_event_count.to_string(),
            time_stop_interval_count: quality.time_stop_interval_count,
            abyss_event_count: quality.abyss_event_count.to_string(),
            server_damage_corrections: quality.server_damage_corrections.to_string(),
            unattributed_server_damage_events: quality
                .unattributed_server_damage_events
                .to_string(),
            unattributed_server_damage: quality.unattributed_server_damage,
        }
    }
}

fn capture_phase_code(phase: LiveCapturePhase) -> &'static str {
    match phase {
        LiveCapturePhase::Idle => "idle",
        LiveCapturePhase::Starting => "starting",
        LiveCapturePhase::Running => "running",
        LiveCapturePhase::Stopping => "stopping",
        LiveCapturePhase::Stopped => "stopped",
        LiveCapturePhase::Failed => "failed",
    }
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use nte_dps_tool::core::diagnostics::{DiagnosticMessage, DiagnosticStatus};

    use super::*;

    #[test]
    fn report_contract_keeps_stable_message_keys_and_omits_private_detail_fields() {
        let snapshot = DiagnosticsReportSnapshot::from(DiagnosticReport {
            checks: vec![DiagnosticCheck {
                status: DiagnosticStatus::Failed,
                title_key: "Game Connection",
                detail: DiagnosticMessage {
                    key: "Recent diagnostic message: {}",
                    arguments: vec!["private detail".to_owned()],
                },
                suggestion: DiagnosticMessage {
                    key: "Address the failed items above, then re-detect",
                    arguments: Vec::new(),
                },
            }],
        });
        let value = serde_json::to_value(snapshot).expect("diagnostics report serializes");

        assert_eq!(value["failedCount"], 1);
        assert_eq!(value["checks"][0]["status"], "failed");
        assert_eq!(
            value["checks"][0]["detail"]["messageKey"],
            "Recent diagnostic message: {}"
        );
        assert!(value["checks"][0].get("detailText").is_none());
    }

    #[test]
    fn action_result_serializes_snapshot_under_stable_field() {
        let state = AppState::default();
        let result = DiagnosticsActionResult {
            performed: false,
            snapshot: DiagnosticsSnapshot::from_state(&state)
                .expect("healthy live-capture diagnostics snapshot"),
        };
        let value = serde_json::to_value(result).expect("action result serializes");

        assert_eq!(value["performed"], false);
        assert_eq!(
            value["snapshot"]["contractVersion"],
            DIAGNOSTICS_CONTRACT_VERSION
        );
        assert!(value["snapshot"]["qualityGeneration"].is_string());
    }

    #[test]
    fn quality_contract_exposes_unattributed_server_damage_without_rounding_event_count() {
        let snapshot = DiagnosticsQualitySnapshot::from(CaptureQualitySummary {
            unattributed_server_damage_events: u64::MAX,
            unattributed_server_damage: 26_955.5,
            ..CaptureQualitySummary::default()
        });
        let value = serde_json::to_value(snapshot).expect("diagnostics quality serializes");

        assert_eq!(
            value["unattributedServerDamageEvents"],
            u64::MAX.to_string()
        );
        assert_eq!(value["unattributedServerDamage"], 26_955.5);
    }
}
