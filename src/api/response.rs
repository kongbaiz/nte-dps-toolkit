use serde::Serialize;

use super::{DATA_VERSION, PROTOCOL_VERSION};

#[derive(Debug, Serialize)]
pub struct VersionResult {
    pub core_version: &'static str,
    pub protocol_version: u32,
    pub data_version: &'static str,
}

impl Default for VersionResult {
    fn default() -> Self {
        Self {
            core_version: env!("CARGO_PKG_VERSION"),
            protocol_version: PROTOCOL_VERSION,
            data_version: DATA_VERSION,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct HelloResult {
    pub core_version: &'static str,
    pub protocol_version: u32,
    pub data_version: &'static str,
    pub capabilities: [&'static str; 4],
    pub raw_capture_default: bool,
}

impl Default for HelloResult {
    fn default() -> Self {
        Self {
            core_version: env!("CARGO_PKG_VERSION"),
            protocol_version: PROTOCOL_VERSION,
            data_version: DATA_VERSION,
            capabilities: ["capture", "inventory", "battle_summary", "equipment"],
            raw_capture_default: true,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct StatusResult {
    pub handshaken: bool,
    pub capture_running: bool,
    pub capture_profile: Option<&'static str>,
    pub core_state: &'static str,
    pub latest_inventory_generation: Option<u64>,
    pub has_battle_data: bool,
    pub raw_capture_path: Option<String>,
}

impl StatusResult {
    pub fn new(
        handshaken: bool,
        capture_running: bool,
        capture_profile: Option<&'static str>,
        latest_inventory_generation: Option<u64>,
        has_battle_data: bool,
        raw_capture_path: Option<String>,
    ) -> Self {
        Self {
            handshaken,
            capture_running,
            capture_profile,
            core_state: if capture_running { "capturing" } else { "idle" },
            latest_inventory_generation,
            has_battle_data,
            raw_capture_path,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ShutdownResult {
    pub shutting_down: bool,
}

#[derive(Debug, Serialize)]
pub struct CaptureStartResult {
    pub operation_id: String,
}

#[derive(Debug, Serialize)]
pub struct CaptureStopResult {
    pub operation_id: String,
    pub stopped: bool,
}

#[derive(Debug, Serialize)]
pub struct EquipmentRequestResult {
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct CaptureStatusEvent {
    pub sequence: u64,
    pub operation_id: String,
    pub status: &'static str,
    pub profile: &'static str,
}

#[derive(Debug, Serialize)]
pub struct CoreMessageEvent {
    pub sequence: u64,
    pub message: &'static str,
}

#[derive(Debug, Serialize)]
pub struct BattleResetResult {
    pub reset: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_preserves_existing_capability_order_and_appends_equipment() {
        assert_eq!(
            HelloResult::default().capabilities,
            ["capture", "inventory", "battle_summary", "equipment"]
        );
    }
}
