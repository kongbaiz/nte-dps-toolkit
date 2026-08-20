//! Frontend-neutral capture diagnostics.
//!
//! Checks expose stable English message keys plus arguments. The Tauri
//! frontend translates those messages at its contract boundary.

use super::capture::{
    AutoDeviceResolution, enumerate_devices, probe_auto_device, resolve_manual_device,
};
use crate::engine::capture::CaptureDevice;
use crate::engine::model::{CaptureQualitySource, CaptureQualitySummary, CombatState};
use crate::platform::network::{
    GameNetwork, GameNetworkProbe, GameNetworkUnavailable, NetworkProbeFailure,
};
use crate::storage::i18n::{self, LocaleLoadDiagnostic};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiagnosticStatus {
    Passed,
    #[default]
    Warning,
    Failed,
}

impl DiagnosticStatus {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Warning => "warning",
            Self::Failed => "failed",
        }
    }

    pub const fn label_key(self) -> &'static str {
        match self {
            Self::Passed => "Passed",
            Self::Warning => "Warning",
            Self::Failed => "Failed",
        }
    }

    const fn rank(self) -> u8 {
        match self {
            Self::Failed => 0,
            Self::Warning => 1,
            Self::Passed => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticMessage {
    pub key: &'static str,
    pub arguments: Vec<String>,
}

impl DiagnosticMessage {
    fn plain(key: &'static str) -> Self {
        Self {
            key,
            arguments: Vec::new(),
        }
    }

    fn with_arguments(key: &'static str, arguments: Vec<String>) -> Self {
        Self { key, arguments }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticCheck {
    pub status: DiagnosticStatus,
    pub title_key: &'static str,
    pub detail: DiagnosticMessage,
    pub suggestion: DiagnosticMessage,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiagnosticReport {
    pub checks: Vec<DiagnosticCheck>,
}

impl DiagnosticReport {
    pub fn failed_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|check| check.status == DiagnosticStatus::Failed)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|check| check.status == DiagnosticStatus::Warning)
            .count()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiagnosticEnvironment {
    pub device_label: Option<String>,
    pub manual_device: bool,
    pub local_ip: Option<String>,
    pub game_connection: Option<DiagnosticGameConnection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticGameConnection {
    pub pid: u32,
    pub local_ip: String,
    pub remote_ip: String,
    pub remote_port: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiagnosticRun {
    pub environment: DiagnosticEnvironment,
    pub report: DiagnosticReport,
}

#[derive(Clone, Debug, Default)]
pub struct DiagnosticSnapshot {
    pub capture_running: bool,
    pub replay_running: bool,
    pub active_capture_filter: Option<String>,
    pub raw_packet_count: usize,
    pub parsed_packet_count: usize,
    pub hit_count: usize,
    pub dropped_history_archives: u64,
    pub include_incoming: bool,
    pub server_damage_calibration: bool,
    pub last_diagnostic: Option<String>,
    pub manual_capture_device: Option<String>,
}

/// Lightweight hand-off for the diagnostics service. Hit attribution is now
/// maintained incrementally inside `CombatState`; preparing this value never
/// walks or clones the authoritative, unbounded hit log.
#[derive(Debug, Default)]
pub(crate) struct DiagnosticQualityCache;

#[derive(Debug)]
pub(crate) struct DiagnosticQualityInput {
    summary: CaptureQualitySummary,
}

impl DiagnosticQualityCache {
    /// Reads only scalar and bounded-index fields while the authoritative state
    /// is borrowed. `session_generation` remains part of the call contract so
    /// callers cannot accidentally reintroduce cross-session cache reuse.
    pub(crate) fn prepare(
        &self,
        state: &CombatState,
        source: CaptureQualitySource,
        _session_generation: u64,
    ) -> DiagnosticQualityInput {
        DiagnosticQualityInput {
            summary: state.capture_quality_summary(source),
        }
    }

    pub(crate) fn finish(&mut self, input: DiagnosticQualityInput) -> CaptureQualitySummary {
        input.summary
    }
}

pub fn run_capture_diagnostics(snapshot: DiagnosticSnapshot) -> DiagnosticRun {
    let mut checks = Vec::new();
    let mut environment = DiagnosticEnvironment {
        manual_device: snapshot.manual_capture_device.is_some(),
        ..DiagnosticEnvironment::default()
    };

    match enumerate_devices() {
        Ok(devices) if devices.is_empty() => {
            checks.push(check(
                DiagnosticStatus::Failed,
                "Npcap Device",
                message("Npcap is loaded but returned no usable capture devices"),
                message("Confirm Npcap is fully installed, and try running as administrator"),
            ));
            checks.push(check(
                DiagnosticStatus::Failed,
                "Game Connection",
                message("No matching capture device, cannot locate the game connection"),
                message("Fix Npcap device enumeration first, then enter a game scene and re-run diagnostics"),
            ));
        }
        Ok(devices) => {
            checks.push(check(
                DiagnosticStatus::Passed,
                "Npcap Device",
                formatted(
                    "Detected {} usable devices",
                    vec![devices.len().to_string()],
                ),
                message("Device enumeration is working"),
            ));

            match snapshot.manual_capture_device.as_deref() {
                Some(name) => match resolve_manual_device(&devices, name) {
                    Ok((index, probe)) => {
                        environment.device_label = Some(device_label(&devices[index]));
                        let (connection, probe_check) = diagnostic_game_probe(probe, true);
                        apply_diagnostic_connection(&mut environment, connection);
                        checks.push(probe_check);
                    }
                    Err(_) => checks.push(check(
                        DiagnosticStatus::Failed,
                        "Game Connection",
                        message("No matching capture device, cannot locate the game connection"),
                        message("Fix Npcap device enumeration first, then enter a game scene and re-run diagnostics"),
                    )),
                },
                None => match probe_auto_device(&devices) {
                    AutoDeviceResolution::Resolved {
                        device_index,
                        network,
                    } => {
                        environment.device_label = Some(device_label(&devices[device_index]));
                        let (connection, probe_check) = diagnostic_game_probe(
                            GameNetworkProbe::Connected(network),
                            false,
                        );
                        apply_diagnostic_connection(&mut environment, connection);
                        checks.push(probe_check);
                    }
                    AutoDeviceResolution::NotConnected(unavailable) => {
                        let (connection, probe_check) = diagnostic_game_probe(
                            GameNetworkProbe::NotConnected(unavailable),
                            false,
                        );
                        apply_diagnostic_connection(&mut environment, connection);
                        checks.push(probe_check);
                    }
                    AutoDeviceResolution::ProbeFailed(failure) => {
                        let (connection, probe_check) = diagnostic_game_probe(
                            GameNetworkProbe::ProbeFailed(failure),
                            false,
                        );
                        apply_diagnostic_connection(&mut environment, connection);
                        checks.push(probe_check);
                    }
                    AutoDeviceResolution::DeviceNotFound { network } => {
                        apply_diagnostic_connection(
                            &mut environment,
                            Some(DiagnosticGameConnection::from(network)),
                        );
                        checks.push(check(
                            DiagnosticStatus::Failed,
                            "Game Connection",
                            message("No matching capture device, cannot locate the game connection"),
                            message("Confirm Npcap is fully installed, and try running as administrator"),
                        ));
                    }
                },
            }
        }
        Err(_) => {
            checks.push(check(
                DiagnosticStatus::Failed,
                "Npcap Device",
                message("Npcap device enumeration failed"),
                message("Install Npcap and confirm WinPcap API-compatible Mode is available"),
            ));
            checks.push(check(
                DiagnosticStatus::Failed,
                "Game Connection",
                message("Npcap unavailable, skipping game-connection lookup"),
                message("Fix the Npcap loading issue first, then re-run diagnostics"),
            ));
        }
    }

    append_runtime_checks(&mut checks, &snapshot);
    append_locale_resource_check(&mut checks, i18n::locale_load_diagnostic());
    checks.sort_by(|left, right| {
        left.status
            .rank()
            .cmp(&right.status.rank())
            .then_with(|| left.title_key.cmp(right.title_key))
    });
    DiagnosticRun {
        environment,
        report: DiagnosticReport { checks },
    }
}

fn device_label(device: &CaptureDevice) -> String {
    if device.description.trim().is_empty() {
        device.name.clone()
    } else {
        device.description.clone()
    }
}

impl From<GameNetwork> for DiagnosticGameConnection {
    fn from(network: GameNetwork) -> Self {
        Self {
            pid: network.pid,
            local_ip: network.local_ip.to_string(),
            remote_ip: network.remote_ip.to_string(),
            remote_port: network.remote_port,
        }
    }
}

fn apply_diagnostic_connection(
    environment: &mut DiagnosticEnvironment,
    connection: Option<DiagnosticGameConnection>,
) {
    environment.local_ip = connection
        .as_ref()
        .map(|connection| connection.local_ip.clone());
    environment.game_connection = connection;
}

fn diagnostic_game_probe(
    probe: GameNetworkProbe,
    manual_device: bool,
) -> (Option<DiagnosticGameConnection>, DiagnosticCheck) {
    match probe {
        GameNetworkProbe::Connected(network) => {
            let pid = network.pid;
            (
                Some(DiagnosticGameConnection::from(network)),
                check(
                    DiagnosticStatus::Passed,
                    "Game Connection",
                    formatted("Located HTGame.exe PID {}", vec![pid.to_string()]),
                    message("Detected an active HTGame.exe connection and matching NIC"),
                ),
            )
        }
        GameNetworkProbe::NotConnected(GameNetworkUnavailable::ProcessNotFound) => (
            None,
            check(
                DiagnosticStatus::Warning,
                "Game Connection",
                formatted(
                    "Game process {} not detected",
                    vec!["HTGame.exe".to_owned()],
                ),
                message(
                    "No active HTGame.exe connection detected; enter a game scene before starting capture",
                ),
            ),
        ),
        GameNetworkProbe::NotConnected(GameNetworkUnavailable::NoUsableConnection { pid }) => (
            None,
            check(
                DiagnosticStatus::Warning,
                "Game Connection",
                formatted(
                    "Detected {} (PID {}) but no usable IPv4 TCP connection for NIC lookup yet",
                    vec!["HTGame.exe".to_owned(), pid.to_string()],
                ),
                message(
                    "Enter a game scene before starting capture, or verify the manually selected NIC",
                ),
            ),
        ),
        GameNetworkProbe::ProbeFailed(failure) => {
            (None, diagnostic_probe_failure(failure, manual_device))
        }
    }
}

fn diagnostic_probe_failure(failure: NetworkProbeFailure, manual_device: bool) -> DiagnosticCheck {
    check(
        if manual_device {
            DiagnosticStatus::Warning
        } else {
            DiagnosticStatus::Failed
        },
        "Game Connection",
        formatted(
            "Game process check failed: {}",
            vec![failure.code.as_str().to_owned()],
        ),
        message("Address the failed items above, then re-detect"),
    )
}

fn append_runtime_checks(checks: &mut Vec<DiagnosticCheck>, snapshot: &DiagnosticSnapshot) {
    if snapshot.dropped_history_archives > 0 {
        checks.push(check(
            DiagnosticStatus::Warning,
            "History Archive",
            formatted(
                "Dropped {} automatic history archive(s)",
                vec![snapshot.dropped_history_archives.to_string()],
            ),
            message("Automatic history archives were dropped after the retry queue filled"),
        ));
    }
    if snapshot.capture_running {
        checks.push(check(
            DiagnosticStatus::Passed,
            "Capture Status",
            snapshot.active_capture_filter.as_deref().map_or_else(
                || message("Live capture started, BPF is being determined"),
                |filter| formatted("Live capture started, BPF={}", vec![filter.to_owned()]),
            ),
            message("A live capture task is running"),
        ));
    } else if snapshot.replay_running {
        checks.push(check(
            DiagnosticStatus::Passed,
            "Capture Status",
            message("Importing a replay"),
            message("Replay import in progress; live-capture checks do not apply"),
        ));
    } else {
        checks.push(check(
            DiagnosticStatus::Warning,
            "Capture Status",
            message("No live capture task right now"),
            message("Run diagnostics after clicking Start to see BPF and raw-capture write status"),
        ));
    }

    if snapshot.raw_packet_count > 0 {
        checks.push(check(
            DiagnosticStatus::Passed,
            "Raw Capture",
            formatted(
                "Wrote {} raw packets",
                vec![snapshot.raw_packet_count.to_string()],
            ),
            message("Raw PCAPNG writing is working"),
        ));
    } else if snapshot.capture_running {
        checks.push(check(
            DiagnosticStatus::Warning,
            "Raw Capture",
            message("Capture is running but no raw packets written yet"),
            message("Confirm the game is in an online scene; narrow or reset the BPF if needed"),
        ));
    } else {
        checks.push(check(
            DiagnosticStatus::Warning,
            "Raw Capture",
            message("No raw packets available right now"),
            message(
                "Start capture and wait for the game to produce network traffic, then re-check",
            ),
        ));
    }

    if snapshot.hit_count > 0 {
        checks.push(check(
            DiagnosticStatus::Passed,
            "Damage Parsing",
            formatted(
                "Parsed {} damage records",
                vec![snapshot.hit_count.to_string()],
            ),
            message("Damage parsing already has results"),
        ));
    } else if snapshot.parsed_packet_count > 0 || snapshot.raw_packet_count > 0 {
        checks.push(check(
            DiagnosticStatus::Warning,
            "Damage Parsing",
            formatted(
                "{} parsed packets but no damage yet",
                vec![snapshot.parsed_packet_count.to_string()],
            ),
            message("Enter combat and deal damage; if still 0, import a PCAPNG to the diagnostics page to review"),
        ));
    } else {
        checks.push(check(
            DiagnosticStatus::Warning,
            "Damage Parsing",
            message("No packet or damage data yet"),
            message("Enter a game scene and start capture first, and confirm the status bar no longer reports capture errors"),
        ));
    }

    checks.push(check(
        if snapshot.include_incoming {
            DiagnosticStatus::Passed
        } else {
            DiagnosticStatus::Warning
        },
        "Incoming Records",
        message(if snapshot.include_incoming {
            "Incoming parsing is enabled"
        } else {
            "Incoming parsing is disabled"
        }),
        message(if snapshot.include_incoming {
            "Incoming stats will be included in the parse-quality report"
        } else {
            "To investigate direction detection, enable incoming records and re-capture"
        }),
    ));

    checks.push(check(
        DiagnosticStatus::Passed,
        "Server Calibration",
        message(if snapshot.server_damage_calibration {
            "Server-side HP delta calibration is enabled"
        } else {
            "Server-side HP delta calibration is disabled"
        }),
        message(if snapshot.server_damage_calibration {
            "Enabled calibration may replace the only recent candidate hit with the observed server HP delta"
        } else {
            "Disabled calibration reports unexplained server HP residuals without changing DPS totals"
        }),
    ));

    if let Some(diagnostic) = snapshot
        .last_diagnostic
        .as_ref()
        .filter(|diagnostic| !diagnostic.trim().is_empty())
    {
        checks.push(check(
            DiagnosticStatus::Warning,
            "Recent Diagnostic",
            formatted("Recent diagnostic message: {}", vec![diagnostic.clone()]),
            message("Address the failed items above, then re-detect"),
        ));
    }
}

fn append_locale_resource_check(
    checks: &mut Vec<DiagnosticCheck>,
    diagnostic: Option<LocaleLoadDiagnostic>,
) {
    let Some(diagnostic) = diagnostic else {
        return;
    };
    checks.push(check(
        DiagnosticStatus::Warning,
        "Localization Resource",
        formatted(
            "Localization fallback is active: {}",
            vec![diagnostic.code().to_owned()],
        ),
        message("Repair or reinstall the language resources, then restart the app"),
    ));
}

fn message(key: &'static str) -> DiagnosticMessage {
    DiagnosticMessage::plain(key)
}

fn formatted(key: &'static str, arguments: Vec<String>) -> DiagnosticMessage {
    DiagnosticMessage::with_arguments(key, arguments)
}

fn check(
    status: DiagnosticStatus,
    title_key: &'static str,
    detail: DiagnosticMessage,
    suggestion: DiagnosticMessage,
) -> DiagnosticCheck {
    DiagnosticCheck {
        status,
        title_key,
        detail,
        suggestion,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::model::{Hit, HitCharacterSource, HitDirection, PacketObservation};
    use crate::platform::network::NetworkProbeErrorCode;

    fn quality_hit(
        timestamp: f64,
        char_id: u32,
        direction: HitDirection,
        char_known: bool,
        damage: f64,
    ) -> Hit {
        Hit {
            timestamp,
            char_id,
            char_name: format!("Character {char_id}"),
            char_known,
            damage,
            byte_offset: timestamp as usize,
            bit_shift: 0,
            char_source: HitCharacterSource::Packet,
            direction,
            target_hp_before: 0.0,
            target_hp_after: 0.0,
            target_max_hp: 0.0,
            target_hp_percent: 0.0,
            target_id: None,
            target_name: None,
            target_name_en: None,
            target_name_ja: None,
            target_monster_id: None,
            target_context: Vec::new(),
            gameplay_effect_index: None,
            gameplay_effect_name: None,
            ability_name: None,
            damage_name: None,
            damage_component: None,
            attack_type: None,
            damage_attribute: None,
            follow_up_damage: 0.0,
            follow_up_timestamp: None,
            follow_up_damage_name: None,
            follow_up_attack_type: None,
            follow_up_damage_attribute: None,
        }
    }

    #[test]
    fn runtime_checks_use_stable_keys_and_order_failures_before_warnings() {
        let snapshot = DiagnosticSnapshot {
            replay_running: true,
            parsed_packet_count: 7,
            last_diagnostic: Some("private runtime detail".to_owned()),
            ..DiagnosticSnapshot::default()
        };
        let mut checks = Vec::new();
        append_runtime_checks(&mut checks, &snapshot);
        checks.sort_by_key(|check| check.status.rank());

        assert_eq!(checks[0].status, DiagnosticStatus::Warning);
        assert!(checks.iter().any(|check| {
            check.title_key == "Capture Status" && check.detail.key == "Importing a replay"
        }));
        assert!(checks.iter().any(|check| {
            check.title_key == "Recent Diagnostic"
                && check.detail.arguments == ["private runtime detail"]
        }));
    }

    #[test]
    fn dropped_history_archives_are_exposed_as_a_runtime_warning() {
        let snapshot = DiagnosticSnapshot {
            dropped_history_archives: 3,
            ..DiagnosticSnapshot::default()
        };
        let mut checks = Vec::new();
        append_runtime_checks(&mut checks, &snapshot);

        let archive = checks
            .iter()
            .find(|check| check.title_key == "History Archive")
            .expect("history archive warning");
        assert_eq!(archive.status, DiagnosticStatus::Warning);
        assert_eq!(archive.detail.arguments, ["3"]);
    }

    #[test]
    fn locale_degradation_uses_one_bounded_stable_diagnostic() {
        let mut checks = Vec::new();
        append_locale_resource_check(&mut checks, Some(LocaleLoadDiagnostic::InvalidJson));

        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, DiagnosticStatus::Warning);
        assert_eq!(checks[0].title_key, "Localization Resource");
        assert_eq!(checks[0].detail.arguments, ["resource_invalid_json"]);
        assert!(checks[0].detail.arguments[0].len() < 64);
        append_locale_resource_check(&mut checks, None);
        assert_eq!(checks.len(), 1);
    }

    #[test]
    fn game_probe_checks_distinguish_normal_negative_from_probe_failure() {
        let (_, process_missing) = diagnostic_game_probe(
            GameNetworkProbe::NotConnected(GameNetworkUnavailable::ProcessNotFound),
            false,
        );
        assert_eq!(process_missing.status, DiagnosticStatus::Warning);
        assert_eq!(process_missing.detail.key, "Game process {} not detected");
        assert_eq!(process_missing.detail.arguments, ["HTGame.exe"]);

        let (_, connection_missing) = diagnostic_game_probe(
            GameNetworkProbe::NotConnected(GameNetworkUnavailable::NoUsableConnection { pid: 42 }),
            false,
        );
        assert_eq!(connection_missing.status, DiagnosticStatus::Warning);
        assert_eq!(
            connection_missing.detail.key,
            "Detected {} (PID {}) but no usable IPv4 TCP connection for NIC lookup yet"
        );
        assert_eq!(connection_missing.detail.arguments, ["HTGame.exe", "42"]);

        let failure = NetworkProbeFailure::new(
            NetworkProbeErrorCode::TcpTableQueryFailed,
            "private OS detail",
        );
        let (_, automatic_failure) =
            diagnostic_game_probe(GameNetworkProbe::ProbeFailed(failure.clone()), false);
        assert_eq!(automatic_failure.status, DiagnosticStatus::Failed);
        assert_eq!(
            automatic_failure.detail.key,
            "Game process check failed: {}"
        );
        assert_eq!(
            automatic_failure.detail.arguments,
            ["TCP_TABLE_QUERY_FAILED"]
        );
        assert!(
            !automatic_failure
                .detail
                .arguments
                .iter()
                .any(|argument| argument.contains("private"))
        );

        let (_, manual_failure) =
            diagnostic_game_probe(GameNetworkProbe::ProbeFailed(failure), true);
        assert_eq!(manual_failure.status, DiagnosticStatus::Warning);
    }

    #[test]
    fn diagnostics_quality_handoff_matches_incremental_engine_summary() {
        let source = CaptureQualitySource::PcapngReplay;
        let mut state = CombatState::default();
        let mut first = quality_hit(1.0, 7, HitDirection::Unknown, false, 100.0);
        first.attack_type = Some("raw category".to_owned());
        first.gameplay_effect_index = Some(11);
        state.push_hit(first);
        state.observe_packet(PacketObservation { parsed_hits: 1 });

        let mut cache = DiagnosticQualityCache;
        let initial_input = cache.prepare(&state, source, 1);
        assert_eq!(
            cache.finish(initial_input),
            state.capture_quality_summary(source)
        );

        state.observe_packet(PacketObservation { parsed_hits: 0 });
        let packet_input = cache.prepare(&state, source, 1);
        assert_eq!(
            cache.finish(packet_input),
            state.capture_quality_summary(source),
            "packet-only counters must change without rebuilding hit attribution"
        );

        let mut second = quality_hit(2.0, 7, HitDirection::Outgoing, false, 50.0);
        second.attack_type = Some("raw category".to_owned());
        second.gameplay_effect_index = Some(11);
        state.push_hit(second);
        state.push_hit(quality_hit(3.0, 9, HitDirection::Incoming, false, 25.0));
        let mut mapped = quality_hit(4.0, 10, HitDirection::Outgoing, true, 75.0);
        mapped.ability_name = Some("mapped ability".to_owned());
        mapped.gameplay_effect_index = Some(12);
        state.push_hit(mapped);

        let append_input = cache.prepare(&state, source, 1);
        assert_eq!(
            cache.finish(append_input),
            state.capture_quality_summary(source),
            "incremental unknown counters must preserve the established quality contract"
        );

        let replacement = CombatState::default();
        let replacement_input = cache.prepare(&replacement, source, 2);
        assert_eq!(
            cache.finish(replacement_input),
            replacement.capture_quality_summary(source),
            "session generation must prevent reuse across replacement state"
        );
    }
}
