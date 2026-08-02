use serde::Serialize;

use nte_dps_tool::core::{
    live_capture::LiveCapturePhase,
    packets::{PACKETS_DISPLAY_LIMIT, PacketsProjection},
};

pub(crate) const PACKETS_CONTRACT_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PacketsSnapshot {
    pub contract_version: u32,
    pub generation: String,
    pub session_generation: String,
    pub packet_generation: String,
    pub capture_phase: &'static str,
    pub event_count: usize,
    pub observed_packet_count: String,
    pub packets_with_hits: String,
    pub retained_packet_count: usize,
    pub queued_event_count: usize,
    pub display_limit: usize,
    pub packets: Vec<PacketSnapshot>,
}

impl PacketsSnapshot {
    pub(crate) fn from_projection(
        projection: PacketsProjection,
        capture_phase: LiveCapturePhase,
    ) -> Self {
        Self {
            contract_version: PACKETS_CONTRACT_VERSION,
            generation: projection.generation.to_string(),
            session_generation: projection.session_generation.to_string(),
            packet_generation: projection.packet_generation.to_string(),
            capture_phase: capture_phase_code(capture_phase),
            event_count: projection.event_count,
            observed_packet_count: projection.observed_packet_count.to_string(),
            packets_with_hits: projection.packets_with_hits.to_string(),
            retained_packet_count: projection.retained_packet_count,
            queued_event_count: projection.queued_event_count,
            display_limit: PACKETS_DISPLAY_LIMIT,
            packets: projection
                .packets
                .into_iter()
                .map(|packet| PacketSnapshot {
                    sequence: packet.sequence.to_string(),
                    timestamp: packet.timestamp,
                    source: packet.source,
                    destination: packet.destination,
                    direction: packet.direction,
                    payload_len: packet.payload_len,
                    declared_ids: packet.declared_ids,
                    parsed_hits: packet.parsed_hits,
                    note: packet.note,
                    decoded_text: packet.decoded_text,
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PacketSnapshot {
    pub sequence: String,
    pub timestamp: f64,
    pub source: String,
    pub destination: String,
    pub direction: String,
    pub payload_len: usize,
    pub declared_ids: Vec<u32>,
    pub parsed_hits: usize,
    pub note: String,
    pub decoded_text: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", content = "payload", rename_all = "camelCase")]
pub(crate) enum PacketsEvent {
    Snapshot(PacketsSnapshot),
    Append(PacketsSnapshot),
}

const fn capture_phase_code(phase: LiveCapturePhase) -> &'static str {
    match phase {
        LiveCapturePhase::Idle => "idle",
        LiveCapturePhase::Starting => "starting",
        LiveCapturePhase::Running => "running",
        LiveCapturePhase::Stopping => "stopping",
        LiveCapturePhase::Stopped => "stopped",
        LiveCapturePhase::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nte_dps_tool::core::packets::{PacketProjection, PacketsProjection};

    #[test]
    fn packet_contract_uses_decimal_strings_and_omits_raw_payload_fields() {
        let snapshot = PacketsSnapshot::from_projection(
            PacketsProjection {
                generation: u64::MAX,
                session_generation: 2,
                packet_generation: 3,
                event_count: 4,
                observed_packet_count: 5,
                packets_with_hits: 1,
                retained_packet_count: 1,
                queued_event_count: 0,
                packets: vec![PacketProjection {
                    sequence: u64::MAX,
                    timestamp: 1.25,
                    source: "source".to_owned(),
                    destination: "destination".to_owned(),
                    direction: "outgoing".to_owned(),
                    payload_len: 128,
                    declared_ids: vec![1076],
                    parsed_hits: 1,
                    note: "note".to_owned(),
                    decoded_text: "decoded".to_owned(),
                }],
            },
            LiveCapturePhase::Running,
        );
        let value = serde_json::to_value(snapshot).expect("packet snapshot serializes");

        assert_eq!(value["generation"], u64::MAX.to_string());
        assert_eq!(value["packets"][0]["sequence"], u64::MAX.to_string());
        assert_eq!(value["capturePhase"], "running");
        assert!(value["packets"][0].get("payloadHex").is_none());
        assert!(value["packets"][0].get("payloadPreview").is_none());
    }
}
