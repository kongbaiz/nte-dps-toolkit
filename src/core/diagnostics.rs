//! Frontend-neutral capture diagnostics.
//!
//! Checks expose stable English message keys plus arguments. The egui and
//! Tauri frontends translate those messages at their respective boundaries.

use super::capture::{enumerate_devices, resolve_auto_device, resolve_manual_device};

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
    pub include_incoming: bool,
    pub server_damage_calibration: bool,
    pub last_diagnostic: Option<String>,
    pub manual_capture_device: Option<String>,
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

            let resolved = match snapshot.manual_capture_device.as_deref() {
                Some(name) => resolve_manual_device(&devices, name)
                    .map(|(index, network)| (index, network.ok())),
                None => {
                    resolve_auto_device(&devices).map(|(index, network)| (index, Some(network)))
                }
            };
            match resolved {
                Ok((index, network)) => {
                    let device = &devices[index];
                    environment.device_label = Some(if device.description.trim().is_empty() {
                        device.name.clone()
                    } else {
                        device.description.clone()
                    });
                    if let Some(network) = network {
                        environment.local_ip = Some(network.local_ip.to_string());
                        environment.game_connection = Some(DiagnosticGameConnection {
                            pid: network.pid,
                            local_ip: network.local_ip.to_string(),
                            remote_ip: network.remote_ip.to_string(),
                            remote_port: network.remote_port,
                        });
                        checks.push(check(
                            DiagnosticStatus::Passed,
                            "Game Connection",
                            formatted("Located HTGame.exe PID {}", vec![network.pid.to_string()]),
                            message("Detected an active HTGame.exe connection and matching NIC"),
                        ));
                    } else {
                        checks.push(check(
                            DiagnosticStatus::Warning,
                            "Game Connection",
                            message("No active HTGame.exe connection detected"),
                            message("Enter a game scene before starting capture, or verify the manually selected NIC"),
                        ));
                    }
                }
                Err(_) => checks.push(check(
                    DiagnosticStatus::Failed,
                    "Game Connection",
                    message("No active HTGame.exe connection detected"),
                    message("No active HTGame.exe connection detected; enter a game scene before starting capture"),
                )),
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

fn append_runtime_checks(checks: &mut Vec<DiagnosticCheck>, snapshot: &DiagnosticSnapshot) {
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
}
