use serde::Serialize;
use ts_rs::TS;

use nte_dps_tool::core::{
    live_capture::LiveCapturePhase,
    packets::{PACKETS_DISPLAY_LIMIT, PacketsProjection},
};

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PacketsSnapshot {
    pub generation: String,
    pub session_generation: String,
    pub packet_generation: String,
    pub first_display_sequence: String,
    pub capture_phase: LiveCapturePhase,
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
            generation: projection.generation.to_string(),
            session_generation: projection.session_generation.to_string(),
            packet_generation: projection.packet_generation.to_string(),
            first_display_sequence: projection.first_display_sequence.to_string(),
            capture_phase,
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

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
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

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(tag = "event", content = "payload", rename_all = "camelCase")]
pub(crate) enum PacketsEvent {
    Snapshot(PacketsSnapshot),
    Append(PacketsSnapshot),
}

#[cfg(test)]
mod tests {
    use super::*;
    use nte_dps_tool::{
        core::packets::{
            PACKETS_DISPLAY_LIMIT, PACKETS_MAX_DECLARED_IDS, PacketStreamRevision,
            project_recent_packets,
        },
        engine::model::{CombatState, PacketDebug},
    };

    use crate::contract::stream::MAX_STREAM_DELIVERY_BYTES;

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

        let replacement = serde_json::to_vec(&PacketsEvent::Snapshot(snapshot.clone()))
            .expect("maximal replacement packet delivery stays serializable");
        let append = serde_json::to_vec(&PacketsEvent::Append(snapshot))
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
