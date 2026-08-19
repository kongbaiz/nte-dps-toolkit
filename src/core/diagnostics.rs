//! Frontend-neutral capture diagnostics.
//!
//! Checks expose stable English message keys plus arguments. The Tauri
//! frontend translates those messages at its contract boundary.

use std::collections::HashSet;

use super::capture::{
    AutoDeviceResolution, enumerate_devices, probe_auto_device, resolve_manual_device,
};
use crate::engine::capture::CaptureDevice;
use crate::engine::model::{
    CaptureQualityScalars, CaptureQualitySource, CaptureQualitySummary, CombatState, Hit,
    HitDirection,
};
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

/// Generation-aware, bounded read model for the expensive hit portion of the
/// diagnostics quality report. Packet-only revisions reuse the aggregate;
/// append-only hit generations extend it; corrections, backfills, trims, and
/// session changes rebuild from the retained hit ring.
///
/// The cache stores at most one key per retained unknown character, unmapped
/// skill row, or unmapped gameplay effect. `CombatState` already bounds that
/// source ring, so this read model inherits the same memory bound.
#[derive(Debug, Default)]
pub(crate) struct DiagnosticQualityCache {
    initialized: bool,
    session_generation: u64,
    hits_generation: u64,
    hit_count: usize,
    hits: DiagnosticHitQuality,
}

#[derive(Debug)]
pub(crate) struct DiagnosticQualityInput {
    source: CaptureQualitySource,
    session_generation: u64,
    scalars: CaptureQualityScalars,
    update: DiagnosticQualityUpdate,
}

#[derive(Debug)]
enum DiagnosticQualityUpdate {
    Reuse,
    Extend(Vec<DiagnosticHitSample>),
    Rebuild(Vec<DiagnosticHitSample>),
}

impl DiagnosticQualityInput {
    #[cfg(test)]
    pub(crate) fn rebuilds_hits(&self) -> bool {
        matches!(self.update, DiagnosticQualityUpdate::Rebuild(_))
    }
}

impl DiagnosticQualityCache {
    /// Copies only compact hit-quality fields while the authoritative state is
    /// borrowed. Hashing, uniqueness maintenance, and aggregate mutation occur
    /// later, after the capture locks have been released.
    pub(crate) fn prepare(
        &self,
        state: &CombatState,
        source: CaptureQualitySource,
        session_generation: u64,
    ) -> DiagnosticQualityInput {
        let scalars = state.capture_quality_scalars();
        let same_session = self.initialized && self.session_generation == session_generation;
        let update = if same_session
            && self.hits_generation == scalars.hits_generation
            && self.hit_count == scalars.hit_count
        {
            DiagnosticQualityUpdate::Reuse
        } else {
            let appended = scalars.hit_count.checked_sub(self.hit_count);
            let generation_delta = scalars.hits_generation.checked_sub(self.hits_generation);
            if same_session
                && appended.is_some_and(|count| count > 0)
                && generation_delta == appended.map(|count| count as u64)
            {
                DiagnosticQualityUpdate::Extend(sample_hits(
                    state.hits.iter().skip(self.hit_count),
                    appended.unwrap_or_default(),
                ))
            } else {
                DiagnosticQualityUpdate::Rebuild(sample_hits(state.hits.iter(), scalars.hit_count))
            }
        };
        DiagnosticQualityInput {
            source,
            session_generation,
            scalars,
            update,
        }
    }

    pub(crate) fn finish(&mut self, input: DiagnosticQualityInput) -> CaptureQualitySummary {
        match input.update {
            DiagnosticQualityUpdate::Reuse => {}
            DiagnosticQualityUpdate::Extend(samples) => self.hits.extend(samples),
            DiagnosticQualityUpdate::Rebuild(samples) => {
                self.hits = DiagnosticHitQuality::from_samples(samples);
            }
        }
        self.initialized = true;
        self.session_generation = input.session_generation;
        self.hits_generation = input.scalars.hits_generation;
        self.hit_count = input.scalars.hit_count;
        self.hits.summary(input.source, input.scalars)
    }
}

#[derive(Debug, Default)]
struct DiagnosticHitQuality {
    outgoing_hits: u64,
    outgoing_damage: f64,
    unknown_direction_hits: u64,
    unknown_direction_damage: f64,
    incoming_hits: u64,
    incoming_damage: f64,
    unknown_characters: HashSet<u32>,
    unknown_character_hits: u64,
    unmapped_skill_rows: HashSet<UnmappedSkillKey>,
    unmapped_skill_hits: u64,
    unmapped_gameplay_effects: HashSet<u32>,
}

impl DiagnosticHitQuality {
    fn from_samples(samples: Vec<DiagnosticHitSample>) -> Self {
        let mut quality = Self::default();
        quality.extend(samples);
        quality
    }

    fn extend(&mut self, samples: Vec<DiagnosticHitSample>) {
        for sample in samples {
            match sample.direction {
                HitDirection::Outgoing => {
                    self.outgoing_hits = self.outgoing_hits.saturating_add(1);
                    self.outgoing_damage += sample.damage;
                }
                HitDirection::Unknown => {
                    self.unknown_direction_hits = self.unknown_direction_hits.saturating_add(1);
                    self.unknown_direction_damage += sample.damage;
                }
                HitDirection::Incoming => {
                    self.incoming_hits = self.incoming_hits.saturating_add(1);
                    self.incoming_damage += sample.damage;
                }
            }
            if let Some(character) = sample.unknown_character {
                self.unknown_characters.insert(character);
                self.unknown_character_hits = self.unknown_character_hits.saturating_add(1);
            }
            if let Some(row) = sample.unmapped_skill_row {
                self.unmapped_skill_rows.insert(row);
            }
            if sample.unmapped_skill_hit {
                self.unmapped_skill_hits = self.unmapped_skill_hits.saturating_add(1);
            }
            if let Some(effect) = sample.unmapped_gameplay_effect {
                self.unmapped_gameplay_effects.insert(effect);
            }
        }
    }

    fn summary(
        &self,
        source: CaptureQualitySource,
        scalars: CaptureQualityScalars,
    ) -> CaptureQualitySummary {
        CaptureQualitySummary {
            source,
            packet_count: scalars.packet_count,
            packets_with_hits: scalars.packets_with_hits,
            hit_count: scalars.hit_count,
            outgoing_hits: self.outgoing_hits,
            outgoing_damage: self.outgoing_damage,
            unknown_direction_hits: self.unknown_direction_hits,
            unknown_direction_damage: self.unknown_direction_damage,
            incoming_hits: self.incoming_hits,
            incoming_damage: self.incoming_damage,
            unknown_character_count: self.unknown_characters.len(),
            unknown_character_hits: self.unknown_character_hits,
            unmapped_skill_rows: self.unmapped_skill_rows.len(),
            unmapped_skill_hits: self.unmapped_skill_hits,
            unmapped_gameplay_effect_count: self.unmapped_gameplay_effects.len(),
            time_stop_event_count: scalars.time_stop_event_count,
            time_stop_interval_count: scalars.time_stop_interval_count,
            abyss_event_count: scalars.abyss_event_count,
            server_damage_corrections: scalars.server_damage_corrections,
        }
    }
}

#[derive(Debug)]
struct DiagnosticHitSample {
    direction: HitDirection,
    damage: f64,
    unknown_character: Option<u32>,
    unmapped_skill_row: Option<UnmappedSkillKey>,
    unmapped_skill_hit: bool,
    unmapped_gameplay_effect: Option<u32>,
}

impl From<&Hit> for DiagnosticHitSample {
    fn from(hit: &Hit) -> Self {
        let contributes_to_attribution = !hit.direction.is_incoming();
        let positive_primary = contributes_to_attribution && hit.damage > 0.0;
        let skill_unmapped = positive_primary
            && hit.damage_name.is_none()
            && hit.damage_component.is_none()
            && hit.ability_name.is_none()
            && hit.gameplay_effect_name.is_none();
        let unmapped_skill_row =
            (skill_unmapped && hit.damage.is_finite()).then(|| UnmappedSkillKey {
                char_id: hit.char_id,
                attack_type: hit.attack_type.clone(),
                gameplay_effect_index: hit.gameplay_effect_index,
            });
        Self {
            direction: hit.direction,
            damage: hit.total_damage(),
            unknown_character: (contributes_to_attribution && !hit.char_known)
                .then_some(hit.char_id),
            unmapped_skill_row,
            unmapped_skill_hit: skill_unmapped,
            unmapped_gameplay_effect: (positive_primary && hit.gameplay_effect_name.is_none())
                .then_some(hit.gameplay_effect_index)
                .flatten(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct UnmappedSkillKey {
    char_id: u32,
    attack_type: Option<String>,
    gameplay_effect_index: Option<u32>,
}

fn sample_hits<'a>(
    hits: impl Iterator<Item = &'a Hit>,
    expected: usize,
) -> Vec<DiagnosticHitSample> {
    let mut samples = Vec::with_capacity(expected);
    samples.extend(hits.map(DiagnosticHitSample::from));
    samples
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
            "Damage values use server-side HP deltas when they can be unambiguously paired"
        } else {
            "To investigate damage-value deviation, enable calibration and re-capture or re-import"
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
    use crate::engine::model::{HitCharacterSource, PacketObservation};
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
    fn cached_quality_projection_matches_legacy_summary_and_extends_append_only_hits() {
        let source = CaptureQualitySource::PcapngReplay;
        let mut state = CombatState::default();
        let mut first = quality_hit(1.0, 7, HitDirection::Unknown, false, 100.0);
        first.attack_type = Some("raw category".to_owned());
        first.gameplay_effect_index = Some(11);
        state.push_hit(first);
        state.observe_packet(PacketObservation { parsed_hits: 1 });

        let mut cache = DiagnosticQualityCache::default();
        let initial_input = cache.prepare(&state, source, 1);
        assert!(initial_input.rebuilds_hits());
        assert_eq!(
            cache.finish(initial_input),
            state.capture_quality_summary(source)
        );

        state.observe_packet(PacketObservation { parsed_hits: 0 });
        let packet_input = cache.prepare(&state, source, 1);
        assert!(!packet_input.rebuilds_hits());
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
        assert!(!append_input.rebuilds_hits());
        assert_eq!(
            cache.finish(append_input),
            state.capture_quality_summary(source),
            "incremental unknown counters must preserve the established quality contract"
        );

        state.hits[0].direction = HitDirection::Outgoing;
        state.hits_generation = state.hits_generation.wrapping_add(1);
        let correction_input = cache.prepare(&state, source, 1);
        assert!(correction_input.rebuilds_hits());
        assert_eq!(
            cache.finish(correction_input),
            state.capture_quality_summary(source),
            "non-append hit mutations must invalidate and rebuild the bounded cache"
        );

        let replacement = CombatState::default();
        let replacement_input = cache.prepare(&replacement, source, 2);
        assert!(replacement_input.rebuilds_hits());
        assert_eq!(
            cache.finish(replacement_input),
            replacement.capture_quality_summary(source),
            "session generation must prevent reuse across replacement state"
        );
    }
}
