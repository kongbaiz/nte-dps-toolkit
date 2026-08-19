//! Non-UI capture preparation and control: game-process probing, Npcap device
//! enumeration, auto/manual NIC resolution, live BPF composition and capture
//! start. Both frontends drive live capture through this module so device
//! selection and filter semantics can never diverge.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{CoreError, CoreErrorCode};
use crate::engine::capture::{
    CaptureDevice, CaptureHandle, CaptureOutput, CaptureResources, EngineEventSink,
    PacketEmissionMode, RawCaptureBuffer, RawCaptureSnapshot, list_devices, start_capture,
};
use crate::engine::model::CharacterInfo;
use crate::engine::parser::AbilityCatalog;
use crate::platform::network::{
    GameNetwork, GameNetworkProbe, GameNetworkUnavailable, NetworkProbeFailure,
    detect_game_network, game_process_is_running,
};
use crate::storage::paths::capture_log_dir;

/// Probe whether the game process is running. `Err` means the OS process
/// query itself failed, not that the game is absent.
pub fn probe_game_process() -> Result<bool, CoreError> {
    game_process_is_running()
        .map_err(|detail| CoreError::new(CoreErrorCode::SystemProbeFailed, detail))
}

pub fn enumerate_devices() -> Result<Vec<CaptureDevice>, CoreError> {
    list_devices().map_err(|detail| CoreError::new(CoreErrorCode::NpcapNotFound, detail))
}

/// Read-only environment snapshot used by CLI discovery. A missing game
/// process or an active process without a usable TCP connection is normal
/// state, not a discovery failure.
pub struct CaptureEnvironment {
    pub game_process_detected: bool,
    pub recommended_device: Option<String>,
    pub local_ip_detected: bool,
    pub devices: Vec<CaptureDevice>,
}

pub fn detect_environment() -> Result<CaptureEnvironment, CoreError> {
    let devices = enumerate_devices()?;
    let (game_process_detected, network) = match detect_game_network() {
        GameNetworkProbe::Connected(network) => (true, Some(network)),
        GameNetworkProbe::NotConnected(GameNetworkUnavailable::ProcessNotFound) => (false, None),
        GameNetworkProbe::NotConnected(GameNetworkUnavailable::NoUsableConnection { .. }) => {
            (true, None)
        }
        GameNetworkProbe::ProbeFailed(failure) => return Err(system_probe_error(failure)),
    };
    let recommended_device = network.as_ref().and_then(|network| {
        devices
            .iter()
            .find(|device| device.ipv4.contains(&network.local_ip))
            .map(|device| device.name.clone())
    });

    Ok(CaptureEnvironment {
        game_process_detected,
        recommended_device,
        local_ip_detected: network.is_some(),
        devices,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutoDeviceResolution {
    Resolved {
        device_index: usize,
        network: GameNetwork,
    },
    NotConnected(GameNetworkUnavailable),
    ProbeFailed(NetworkProbeFailure),
    DeviceNotFound {
        network: GameNetwork,
    },
}

pub fn probe_auto_device(devices: &[CaptureDevice]) -> AutoDeviceResolution {
    classify_auto_device_probe(devices, detect_game_network())
}

fn classify_auto_device_probe(
    devices: &[CaptureDevice],
    probe: GameNetworkProbe,
) -> AutoDeviceResolution {
    match probe {
        GameNetworkProbe::Connected(network) => {
            if let Some(device_index) = devices
                .iter()
                .position(|device| device.ipv4.contains(&network.local_ip))
            {
                AutoDeviceResolution::Resolved {
                    device_index,
                    network,
                }
            } else {
                AutoDeviceResolution::DeviceNotFound { network }
            }
        }
        GameNetworkProbe::NotConnected(unavailable) => {
            AutoDeviceResolution::NotConnected(unavailable)
        }
        GameNetworkProbe::ProbeFailed(failure) => AutoDeviceResolution::ProbeFailed(failure),
    }
}

fn system_probe_error(failure: NetworkProbeFailure) -> CoreError {
    CoreError::new(
        CoreErrorCode::SystemProbeFailed,
        format!("{}: {}", failure.code.as_str(), failure.detail),
    )
}

/// Auto mode requires both an active game connection and a matching Npcap
/// device. Normal negative state and OS probe failure retain distinct typed
/// outcomes until this frontend-neutral error boundary.
pub fn resolve_auto_device(devices: &[CaptureDevice]) -> Result<(usize, GameNetwork), CoreError> {
    match probe_auto_device(devices) {
        AutoDeviceResolution::Resolved {
            device_index,
            network,
        } => Ok((device_index, network)),
        AutoDeviceResolution::NotConnected(unavailable) => Err(CoreError::new(
            CoreErrorCode::GameProcessNotFound,
            unavailable.detail(),
        )),
        AutoDeviceResolution::ProbeFailed(failure) => Err(system_probe_error(failure)),
        AutoDeviceResolution::DeviceNotFound { network } => Err(CoreError::new(
            CoreErrorCode::CaptureDeviceNotFound,
            format!(
                "no Npcap device matches the game's local IP {}",
                network.local_ip
            ),
        )),
    }
}

/// Manual mode: pin capture to the named NIC. The outer error means the NIC
/// vanished (`detail` = the requested name). The typed game-connection probe
/// is best-effort: both a normal miss and a probe failure are non-fatal, but a
/// probe failure remains attached to the controller as a degradation while
/// direction inference falls back to its public/private heuristic.
pub fn resolve_manual_device(
    devices: &[CaptureDevice],
    name: &str,
) -> Result<(usize, GameNetworkProbe), CoreError> {
    let index = devices
        .iter()
        .position(|device| device.name == name)
        .ok_or_else(|| CoreError::new(CoreErrorCode::CaptureDeviceNotFound, name))?;
    Ok((index, detect_game_network()))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ManualNetworkSelection {
    network: Option<GameNetwork>,
    degradation: Option<NetworkProbeFailure>,
}

fn manual_network_selection(probe: GameNetworkProbe) -> ManualNetworkSelection {
    match probe {
        GameNetworkProbe::Connected(network) => ManualNetworkSelection {
            network: Some(network),
            degradation: None,
        },
        GameNetworkProbe::NotConnected(_) => ManualNetworkSelection::default(),
        GameNetworkProbe::ProbeFailed(failure) => ManualNetworkSelection {
            network: None,
            degradation: Some(failure),
        },
    }
}

/// The base filter (`base`, "udp") keeps all UDP, which covers the game-world
/// server that carries combat/GAS replication and equipment (e.g. :30196).
/// The game's account / life-sim service talks TCP :30031 to a *different*
/// server IP, so a UDP-only BPF drops it before it can even reach the raw
/// pcapng. Widen the filter to also keep everything to/from that detected
/// host. The live parser only decodes UDP (`parse_udp_ipv4` rejects non-UDP),
/// so the extra TCP frames are retained for offline analysis without affecting
/// live parsing. Falls back to UDP-only if the game endpoint was not detected.
pub fn compose_bpf(base: &str, network: Option<&GameNetwork>) -> String {
    match network {
        Some(network) => format!("{} or host {}", base, network.remote_ip),
        None => base.to_owned(),
    }
}

pub struct CaptureStartOptions {
    pub device: CaptureDevice,
    pub local_ip: Option<Ipv4Addr>,
    pub filter: String,
    pub include_incoming: bool,
    pub server_damage_calibration: bool,
    pub raw_capture: RawCaptureMode,
    pub packet_emission: PacketEmissionMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureProfile {
    Inventory,
    Combat,
}

impl CaptureProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inventory => "inventory",
            Self::Combat => "combat",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureDeviceSelector {
    Auto,
    Name(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawCaptureMode {
    Enabled,
    Disabled,
}

pub struct CaptureControllerOptions {
    pub profile: CaptureProfile,
    pub device: CaptureDeviceSelector,
    pub filter: String,
    pub include_incoming: bool,
    pub server_damage_calibration: bool,
    pub raw_capture: RawCaptureMode,
    pub raw_capture_directory: PathBuf,
    pub expose_raw_capture_path: bool,
    pub packet_emission: PacketEmissionMode,
}

#[derive(Default)]
pub struct CaptureController {
    capture: Option<CaptureHandle>,
    last_raw_capture: Option<RawCaptureBuffer>,
    profile: Option<CaptureProfile>,
    expose_raw_capture_path: bool,
    active_filter: Option<String>,
    network_probe_degradation: Option<NetworkProbeFailure>,
}

impl CaptureController {
    pub fn is_running(&self) -> bool {
        self.capture.is_some()
    }

    pub fn profile(&self) -> Option<CaptureProfile> {
        self.profile
    }

    pub fn raw_capture_path(&self) -> Option<PathBuf> {
        if !self.expose_raw_capture_path {
            return None;
        }
        self.capture.as_ref()?.raw_capture().path()
    }

    pub fn active_filter(&self) -> Option<String> {
        self.active_filter.clone()
    }

    pub fn network_probe_degradation(&self) -> Option<NetworkProbeFailure> {
        self.network_probe_degradation.clone()
    }

    pub fn raw_capture_snapshot(&self) -> Option<RawCaptureSnapshot> {
        self.capture
            .as_ref()
            .map(|capture| capture.raw_capture())
            .or_else(|| self.last_raw_capture.clone())
            .map(|capture| capture.snapshot())
    }

    pub fn save_last_raw_capture(&self, path: &Path) -> Result<(u64, u64), String> {
        let raw_capture = self
            .last_raw_capture
            .as_ref()
            .ok_or_else(|| "no completed raw capture is available".to_owned())?;
        raw_capture.save(path)
    }

    pub fn start(
        &mut self,
        options: CaptureControllerOptions,
        characters: Arc<HashMap<u32, CharacterInfo>>,
        ability_catalog: Arc<AbilityCatalog>,
        sender: impl Into<EngineEventSink>,
    ) -> Result<(), CoreError> {
        if self.capture.is_some() {
            return Err(CoreError::new(
                CoreErrorCode::CaptureAlreadyRunning,
                "capture is already running",
            ));
        }
        self.network_probe_degradation = None;

        let devices = enumerate_devices()?;
        let (device_index, network, network_probe_degradation) = match &options.device {
            CaptureDeviceSelector::Auto => {
                let (device_index, network) = resolve_auto_device(&devices)?;
                (device_index, Some(network), None)
            }
            CaptureDeviceSelector::Name(name) => {
                let (device_index, probe) = resolve_manual_device(&devices, name)?;
                let selection = manual_network_selection(probe);
                (device_index, selection.network, selection.degradation)
            }
        };
        let device = devices[device_index].clone();
        let local_ip = network.as_ref().map(|network| network.local_ip);
        let filter = compose_bpf(&options.filter, network.as_ref());
        let raw_capture_directory =
            raw_capture_directory(options.raw_capture, &options.raw_capture_directory);
        let capture = start_capture(
            device,
            local_ip,
            filter.clone(),
            options.include_incoming,
            options.server_damage_calibration,
            CaptureResources {
                characters,
                ability_catalog,
            },
            CaptureOutput {
                raw_capture_directory,
                packet_emission: options.packet_emission,
                sender: sender.into(),
            },
        );
        self.capture = Some(capture);
        self.last_raw_capture = None;
        self.profile = Some(options.profile);
        self.expose_raw_capture_path = options.expose_raw_capture_path;
        self.active_filter = Some(filter);
        self.network_probe_degradation = network_probe_degradation;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), CoreError> {
        let Some(mut capture) = self.capture.take() else {
            return Err(CoreError::new(
                CoreErrorCode::CaptureNotRunning,
                "capture is not running",
            ));
        };
        let raw_capture = capture.raw_capture();
        capture.stop();
        self.last_raw_capture = Some(raw_capture);
        self.profile = None;
        self.expose_raw_capture_path = false;
        self.active_filter = None;
        self.network_probe_degradation = None;
        Ok(())
    }

    pub fn stop_if_running(&mut self) {
        if let Some(mut capture) = self.capture.take() {
            let raw_capture = capture.raw_capture();
            capture.stop();
            self.last_raw_capture = Some(raw_capture);
        }
        self.profile = None;
        self.expose_raw_capture_path = false;
        self.active_filter = None;
        self.network_probe_degradation = None;
    }

    pub fn capture_stopped(&mut self) {
        if let Some(capture) = self.capture.take() {
            self.last_raw_capture = Some(capture.raw_capture());
        }
        self.profile = None;
        self.expose_raw_capture_path = false;
        self.active_filter = None;
        self.network_probe_degradation = None;
    }
}

pub fn raw_capture_directory(mode: RawCaptureMode, directory: &Path) -> Option<PathBuf> {
    match mode {
        RawCaptureMode::Enabled => Some(directory.to_owned()),
        RawCaptureMode::Disabled => None,
    }
}

/// Start the live capture thread. Infallible by design: runtime failures
/// surface as `EngineEvent::Error` on `sender`. The returned handle owns the
/// capture thread and the raw-PCAPNG buffer; `stop()`/drop ends the capture.
pub fn start(
    options: CaptureStartOptions,
    characters: Arc<HashMap<u32, CharacterInfo>>,
    ability_catalog: Arc<AbilityCatalog>,
    sender: impl Into<EngineEventSink>,
) -> CaptureHandle {
    start_capture(
        options.device,
        options.local_ip,
        options.filter,
        options.include_incoming,
        options.server_damage_calibration,
        CaptureResources {
            characters,
            ability_catalog,
        },
        CaptureOutput {
            raw_capture_directory: raw_capture_directory(options.raw_capture, &capture_log_dir()),
            packet_emission: options.packet_emission,
            sender: sender.into(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::network::NetworkProbeErrorCode;

    fn device(name: &str) -> CaptureDevice {
        CaptureDevice {
            name: name.to_owned(),
            description: String::new(),
            ipv4: Vec::new(),
        }
    }

    #[test]
    fn compose_bpf_widens_to_detected_host() {
        let network = GameNetwork {
            pid: 1,
            local_ip: "192.168.1.2".parse().unwrap(),
            remote_ip: "203.0.113.9".parse().unwrap(),
            remote_port: 30031,
        };
        assert_eq!(
            compose_bpf("udp", Some(&network)),
            "udp or host 203.0.113.9"
        );
        assert_eq!(compose_bpf("udp", None), "udp");
    }

    #[test]
    fn manual_resolution_reports_missing_nic() {
        let devices = vec![device("a"), device("b")];
        let error = resolve_manual_device(&devices, "gone").unwrap_err();
        assert_eq!(error.code, CoreErrorCode::CaptureDeviceNotFound);
        assert_eq!(error.detail, "gone");
    }

    #[test]
    fn auto_resolution_keeps_normal_negative_and_probe_failure_distinct() {
        let devices = vec![device("a")];
        assert!(matches!(
            classify_auto_device_probe(
                &devices,
                GameNetworkProbe::NotConnected(GameNetworkUnavailable::ProcessNotFound),
            ),
            AutoDeviceResolution::NotConnected(GameNetworkUnavailable::ProcessNotFound)
        ));

        let failure = NetworkProbeFailure::new(
            NetworkProbeErrorCode::TcpTableQueryFailed,
            "fixture TCP failure",
        );
        assert_eq!(
            classify_auto_device_probe(&devices, GameNetworkProbe::ProbeFailed(failure.clone()),),
            AutoDeviceResolution::ProbeFailed(failure)
        );
    }

    #[test]
    fn manual_probe_failure_continues_without_network_and_records_degradation() {
        let failure = NetworkProbeFailure::new(
            NetworkProbeErrorCode::ProcessSnapshotFailed,
            "fixture process failure",
        );

        let selection = manual_network_selection(GameNetworkProbe::ProbeFailed(failure.clone()));

        assert!(selection.network.is_none());
        assert_eq!(selection.degradation, Some(failure));

        let normal_negative = manual_network_selection(GameNetworkProbe::NotConnected(
            GameNetworkUnavailable::NoUsableConnection { pid: 11 },
        ));
        assert!(normal_negative.network.is_none());
        assert!(normal_negative.degradation.is_none());
    }

    #[test]
    fn connected_auto_resolution_requires_a_matching_nic() {
        let mut matching = device("matching");
        matching.ipv4.push("192.168.1.2".parse().unwrap());
        let network = GameNetwork {
            pid: 1,
            local_ip: "192.168.1.2".parse().unwrap(),
            remote_ip: "203.0.113.9".parse().unwrap(),
            remote_port: 30031,
        };

        assert!(matches!(
            classify_auto_device_probe(
                std::slice::from_ref(&matching),
                GameNetworkProbe::Connected(network.clone()),
            ),
            AutoDeviceResolution::Resolved {
                device_index: 0,
                network: resolved,
            } if resolved == network
        ));
        assert!(matches!(
            classify_auto_device_probe(&[], GameNetworkProbe::Connected(network)),
            AutoDeviceResolution::DeviceNotFound { .. }
        ));
    }

    #[test]
    fn raw_capture_mode_preserves_enabled_default_and_disables_explicitly() {
        let directory = Path::new("capture-root");
        assert_eq!(
            raw_capture_directory(RawCaptureMode::Enabled, directory),
            Some(directory.to_owned())
        );
        assert_eq!(
            raw_capture_directory(RawCaptureMode::Disabled, directory),
            None
        );
    }
}
