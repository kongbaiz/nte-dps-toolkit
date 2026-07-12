//! Non-UI capture preparation and control: game-process probing, Npcap device
//! enumeration, auto/manual NIC resolution, live BPF composition and capture
//! start. Both frontends drive live capture through this module so device
//! selection and filter semantics can never diverge.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossbeam_channel::Sender;

use super::{CoreError, CoreErrorCode};
use crate::engine::capture::{
    CaptureDevice, CaptureHandle, CaptureOutput, PacketEmissionMode, list_devices, start_capture,
};
use crate::engine::model::{CharacterInfo, EngineEvent};
use crate::platform::network::{
    GameNetwork, detect_game_device, detect_game_network, game_process_is_running,
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
    let game_process_detected = probe_game_process()?;
    let network = if game_process_detected {
        // The platform probe currently reports "no active connection" and OS
        // lookup failures as the same String. Detection is best-effort, so both
        // remain a soft "local IP unavailable" result until that boundary gains
        // typed errors.
        detect_game_network().ok()
    } else {
        None
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

/// Auto mode: locate the game's active TCP connection and the NIC that owns
/// its local IP. The detail distinguishes "game not detected" from "no NIC
/// carries the game's local IP"; both map to `GameProcessNotFound` because the
/// underlying probe reports them as one opaque message.
pub fn resolve_auto_device(devices: &[CaptureDevice]) -> Result<(usize, GameNetwork), CoreError> {
    detect_game_device(devices)
        .map_err(|detail| CoreError::new(CoreErrorCode::GameProcessNotFound, detail))
}

/// Manual mode: pin capture to the named NIC. The outer error means the NIC
/// vanished (`detail` = the requested name). The inner result is the
/// best-effort game-connection probe: a miss is non-fatal — capture still
/// proceeds and direction inference falls back to its public/private
/// heuristic.
pub fn resolve_manual_device(
    devices: &[CaptureDevice],
    name: &str,
) -> Result<(usize, Result<GameNetwork, CoreError>), CoreError> {
    let index = devices
        .iter()
        .position(|device| device.name == name)
        .ok_or_else(|| CoreError::new(CoreErrorCode::CaptureDeviceNotFound, name))?;
    let network = detect_game_network()
        .map_err(|detail| CoreError::new(CoreErrorCode::GameProcessNotFound, detail));
    Ok((index, network))
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
    profile: Option<CaptureProfile>,
    expose_raw_capture_path: bool,
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

    pub fn start(
        &mut self,
        options: CaptureControllerOptions,
        characters: Arc<HashMap<u32, CharacterInfo>>,
        sender: Sender<EngineEvent>,
    ) -> Result<(), CoreError> {
        if self.capture.is_some() {
            return Err(CoreError::new(
                CoreErrorCode::CaptureAlreadyRunning,
                "capture is already running",
            ));
        }

        let devices = enumerate_devices()?;
        let (device_index, network) = match &options.device {
            CaptureDeviceSelector::Auto => {
                let (device_index, network) = resolve_auto_device(&devices)?;
                (device_index, Some(network))
            }
            CaptureDeviceSelector::Name(name) => {
                let (device_index, network) = resolve_manual_device(&devices, name)?;
                (device_index, network.ok())
            }
        };
        let device = devices[device_index].clone();
        let local_ip = network.as_ref().map(|network| network.local_ip);
        let filter = compose_bpf("udp", network.as_ref());
        let raw_capture_directory =
            raw_capture_directory(options.raw_capture, &options.raw_capture_directory);
        let capture = start_capture(
            device,
            local_ip,
            filter,
            options.include_incoming,
            options.server_damage_calibration,
            characters,
            CaptureOutput {
                raw_capture_directory,
                packet_emission: options.packet_emission,
                sender,
            },
        );
        self.capture = Some(capture);
        self.profile = Some(options.profile);
        self.expose_raw_capture_path = options.expose_raw_capture_path;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), CoreError> {
        let Some(mut capture) = self.capture.take() else {
            return Err(CoreError::new(
                CoreErrorCode::CaptureNotRunning,
                "capture is not running",
            ));
        };
        capture.stop();
        self.profile = None;
        self.expose_raw_capture_path = false;
        Ok(())
    }

    pub fn stop_if_running(&mut self) {
        if let Some(mut capture) = self.capture.take() {
            capture.stop();
        }
        self.profile = None;
        self.expose_raw_capture_path = false;
    }

    pub fn capture_stopped(&mut self) {
        self.capture = None;
        self.profile = None;
        self.expose_raw_capture_path = false;
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
    sender: Sender<EngineEvent>,
) -> CaptureHandle {
    start_capture(
        options.device,
        options.local_ip,
        options.filter,
        options.include_incoming,
        options.server_damage_calibration,
        characters,
        CaptureOutput {
            raw_capture_directory: raw_capture_directory(options.raw_capture, &capture_log_dir()),
            packet_emission: options.packet_emission,
            sender,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
