use serde::Serialize;

use nte_dps_tool::core::{
    live_capture::LiveCapturePhase,
    packets::{PACKETS_DISPLAY_LIMIT, PacketsProjection},
};

pub(crate) const PACKETS_CONTRACT_VERSION: u32 = 2;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PacketsSnapshot {
    pub contract_version: u32,
    pub generation: String,
    pub session_generation: String,
    pub packet_generation: String,
    pub first_display_sequence: String,
    pub capture_phase: &'static str,
    pub event_count: usize,
    pub observed_packet_count: String,
    pub packets_with_hits: String,
    pub retained_packet_count: usize,
    pub queued_event_count: usize,
    pub display_limit: usize,
    pub truncated_packet_count: usize,
    pub omitted_text_bytes: String,
    pub omitted_declared_id_count: String,
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
            first_display_sequence: projection.first_display_sequence.to_string(),
            capture_phase: capture_phase_code(capture_phase),
            event_count: projection.event_count,
            observed_packet_count: projection.observed_packet_count.to_string(),
            packets_with_hits: projection.packets_with_hits.to_string(),
            retained_packet_count: projection.retained_packet_count,
            queued_event_count: projection.queued_event_count,
            display_limit: PACKETS_DISPLAY_LIMIT,
            truncated_packet_count: projection.truncated_packet_count,
            omitted_text_bytes: projection.omitted_text_bytes.to_string(),
            omitted_declared_id_count: projection.omitted_declared_id_count.to_string(),
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
                    omitted_text_bytes: packet.omitted_text_bytes.to_string(),
                    omitted_declared_id_count: packet.omitted_declared_id_count.to_string(),
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
    pub omitted_text_bytes: String,
    pub omitted_declared_id_count: String,
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
    use nte_dps_tool::{
        core::packets::{
            PACKETS_DISPLAY_LIMIT, PACKETS_MAX_DECLARED_IDS, PacketProjection,
            PacketStreamRevision, PacketsProjection, project_recent_packets,
        },
        engine::model::{CombatState, PacketDebug},
    };

    use crate::{
        channels::stream_runtime::serialize_stream_events,
        contract::stream::MAX_STREAM_DELIVERY_BYTES,
    };

    #[test]
    fn packet_contract_uses_decimal_strings_and_omits_raw_payload_fields() {
        let snapshot = PacketsSnapshot::from_projection(
            PacketsProjection {
                generation: u64::MAX,
                session_generation: 2,
                packet_generation: 3,
                first_display_sequence: 1,
                event_count: 4,
                observed_packet_count: 5,
                packets_with_hits: 1,
                retained_packet_count: 1,
                queued_event_count: 0,
                truncated_packet_count: 0,
                omitted_text_bytes: 0,
                omitted_declared_id_count: 0,
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
                    omitted_text_bytes: 0,
                    omitted_declared_id_count: 0,
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

    #[test]
    fn five_hundred_worst_case_escaped_packets_fit_the_stream_envelope() {
        let worst_escape_text = "\0".repeat(4_096);
        let mut state = CombatState::default();
        for _ in 0..PACKETS_DISPLAY_LIMIT {
            assert!(state.push_packet(PacketDebug {
                timestamp: f64::MAX,
                source: "\0".repeat(513),
                destination: "\0".repeat(513),
                direction: "\0".repeat(65),
                payload_len: usize::MAX,
                declared_ids: vec![u32::MAX; PACKETS_MAX_DECLARED_IDS + 44],
                parsed_hits: usize::MAX,
                note: worst_escape_text.clone(),
                payload_preview: String::new(),
                payload_hex: String::new(),
                decoded_text: worst_escape_text.clone(),
            }));
        }
        assert_eq!(state.packets.len(), PACKETS_DISPLAY_LIMIT);
        state.packets_generation = u64::MAX;
        state.packet_count = usize::MAX;
        state.packets_with_hits = usize::MAX;
        let projection = project_recent_packets(
            &state,
            PacketStreamRevision {
                generation: u64::MAX,
                session_generation: u64::MAX,
                packet_generation: state.packets_generation,
                observed_packet_count: state.packet_count,
            },
            usize::MAX,
        );
        assert_eq!(projection.packets.len(), PACKETS_DISPLAY_LIMIT);
        assert_eq!(projection.truncated_packet_count, PACKETS_DISPLAY_LIMIT);
        let snapshot = PacketsSnapshot::from_projection(projection, LiveCapturePhase::Running);

        let replacement = serialize_stream_events(vec![PacketsEvent::Snapshot(snapshot.clone())])
            .expect("maximal replacement packet delivery stays serializable");
        let append = serialize_stream_events(vec![PacketsEvent::Append(snapshot)])
            .expect("maximal append packet delivery stays serializable");
        eprintln!(
            "PACKETS_MAX_STREAM_BYTES replacement={} append={} limit={}",
            replacement.len(),
            append.len(),
            MAX_STREAM_DELIVERY_BYTES
        );
        for bytes in [replacement, append] {
            assert!(bytes.len() < MAX_STREAM_DELIVERY_BYTES);
            assert!(
                bytes.len() < 14 * 1024 * 1024,
                "packet contract must retain at least 2 MiB of stream headroom"
            );
        }
    }
}
