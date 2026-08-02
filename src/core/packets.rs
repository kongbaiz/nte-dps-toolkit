//! Bounded, frontend-neutral projection for the Console packet inspector.
//!
//! Raw payload bytes and payload hex stay inside the engine. The projection
//! exposes only the fields the established packet inspector renders and caps
//! every snapshot/batch before it crosses a frontend boundary.

use crate::engine::model::{CombatState, PacketDebug};

pub const PACKETS_DISPLAY_LIMIT: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketStreamRevision {
    pub generation: u64,
    pub session_generation: u64,
    pub packet_generation: u64,
    pub observed_packet_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PacketsProjection {
    pub generation: u64,
    pub session_generation: u64,
    pub packet_generation: u64,
    pub event_count: usize,
    pub observed_packet_count: usize,
    pub packets_with_hits: usize,
    pub retained_packet_count: usize,
    pub queued_event_count: usize,
    pub packets: Vec<PacketProjection>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PacketProjection {
    pub sequence: u64,
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

pub fn project_recent_packets(
    state: &CombatState,
    revision: PacketStreamRevision,
    queued_event_count: usize,
) -> PacketsProjection {
    let first_sequence = state
        .packets_generation
        .saturating_sub(state.packets.len() as u64)
        .saturating_add(1);
    let display_first_sequence = state
        .packets_generation
        .saturating_sub(PACKETS_DISPLAY_LIMIT.saturating_sub(1) as u64)
        .max(first_sequence);
    projection_from_sequence(state, revision, queued_event_count, display_first_sequence)
}

/// Returns only debug packets newer than `after_packet_generation` when they
/// are still present in the bounded engine ring. `None` requests a full
/// replacement snapshot because the consumer fell behind or changed session.
pub fn project_packets_since(
    state: &CombatState,
    revision: PacketStreamRevision,
    queued_event_count: usize,
    after_packet_generation: u64,
) -> Option<PacketsProjection> {
    if after_packet_generation > state.packets_generation {
        return None;
    }
    let first_retained_sequence = state
        .packets_generation
        .saturating_sub(state.packets.len() as u64)
        .saturating_add(1);
    let requested_first_sequence = after_packet_generation.saturating_add(1);
    if requested_first_sequence < first_retained_sequence
        || state
            .packets_generation
            .saturating_sub(after_packet_generation)
            > PACKETS_DISPLAY_LIMIT as u64
    {
        return None;
    }
    Some(projection_from_sequence(
        state,
        revision,
        queued_event_count,
        requested_first_sequence,
    ))
}

fn projection_from_sequence(
    state: &CombatState,
    revision: PacketStreamRevision,
    queued_event_count: usize,
    first_sequence: u64,
) -> PacketsProjection {
    let retained_first_sequence = state
        .packets_generation
        .saturating_sub(state.packets.len() as u64)
        .saturating_add(1);
    let skip = first_sequence.saturating_sub(retained_first_sequence) as usize;
    let packets = state
        .packets
        .iter()
        .skip(skip.min(state.packets.len()))
        .enumerate()
        .map(|(offset, packet)| {
            PacketProjection::from_packet(first_sequence.saturating_add(offset as u64), packet)
        })
        .collect();
    PacketsProjection {
        generation: revision.generation,
        session_generation: revision.session_generation,
        packet_generation: revision.packet_generation,
        event_count: state.hits.len(),
        observed_packet_count: state.packet_count,
        packets_with_hits: state.packets_with_hits,
        retained_packet_count: state.packets.len(),
        queued_event_count,
        packets,
    }
}

impl PacketProjection {
    fn from_packet(sequence: u64, packet: &PacketDebug) -> Self {
        Self {
            sequence,
            timestamp: packet.timestamp,
            source: packet.source.clone(),
            destination: packet.destination.clone(),
            direction: packet.direction.clone(),
            payload_len: packet.payload_len,
            declared_ids: packet.declared_ids.clone(),
            parsed_hits: packet.parsed_hits,
            note: packet.note.clone(),
            decoded_text: packet.decoded_text.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(index: usize) -> PacketDebug {
        PacketDebug {
            timestamp: index as f64,
            source: format!("source-{index}"),
            destination: format!("destination-{index}"),
            direction: "outgoing".to_owned(),
            payload_len: index,
            declared_ids: vec![index as u32],
            parsed_hits: usize::from(index.is_multiple_of(2)),
            note: format!("note-{index}"),
            payload_preview: "private-preview".to_owned(),
            payload_hex: "deadbeef".to_owned(),
            decoded_text: format!("decoded-{index}"),
        }
    }

    fn revision(state: &CombatState) -> PacketStreamRevision {
        PacketStreamRevision {
            generation: state.packets_generation,
            session_generation: 3,
            packet_generation: state.packets_generation,
            observed_packet_count: state.packet_count,
        }
    }

    #[test]
    fn recent_projection_keeps_only_the_latest_display_window() {
        let mut state = CombatState::default();
        for index in 0..(PACKETS_DISPLAY_LIMIT + 3) {
            state.push_packet(packet(index));
        }

        let projection = project_recent_packets(&state, revision(&state), 7);

        assert_eq!(projection.packets.len(), PACKETS_DISPLAY_LIMIT);
        assert_eq!(projection.packets[0].sequence, 4);
        assert_eq!(projection.packets[0].source, "source-3");
        assert_eq!(projection.queued_event_count, 7);
        assert_eq!(
            projection.packets.last().expect("last").decoded_text,
            "decoded-502"
        );
    }

    #[test]
    fn incremental_projection_returns_only_new_packets() {
        let mut state = CombatState::default();
        for index in 0..5 {
            state.push_packet(packet(index));
        }

        let projection = project_packets_since(&state, revision(&state), 0, 3)
            .expect("consumer remains inside the ring");

        assert_eq!(projection.packets.len(), 2);
        assert_eq!(projection.packets[0].sequence, 4);
        assert_eq!(projection.packets[1].sequence, 5);
    }

    #[test]
    fn incremental_projection_requests_replacement_after_a_large_gap() {
        let mut state = CombatState::default();
        for index in 0..(PACKETS_DISPLAY_LIMIT + 1) {
            state.push_packet(packet(index));
        }

        assert!(project_packets_since(&state, revision(&state), 0, 0).is_none());
    }
}
