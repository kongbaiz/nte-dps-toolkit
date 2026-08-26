//! Bounded, frontend-neutral projection for the Console packet inspector.
//!
//! Raw payload bytes and payload hex stay inside the engine. The projection
//! exposes only the fields the established packet inspector renders and caps
//! every snapshot/batch before it crosses a frontend boundary.

use crate::engine::model::{CombatState, PacketDebug};

pub const PACKETS_DISPLAY_LIMIT: usize = 500;
pub const PACKETS_MAX_SOURCE_BYTES: usize = 512;
pub const PACKETS_MAX_DESTINATION_BYTES: usize = 512;
pub const PACKETS_MAX_DIRECTION_BYTES: usize = 64;
pub const PACKETS_MAX_NOTE_BYTES: usize = 16_384;
pub const PACKETS_MAX_DECODED_TEXT_BYTES: usize = 2_000_000;
pub const PACKETS_MAX_DECLARED_IDS: usize = 256;

/// JSON-escaped string content budget for one replacement or append snapshot.
/// The remaining stream budget is reserved for 500 packet objects, 256 u32 IDs
/// per packet, decimal counters, the event tag, and the stream envelope.
pub const PACKETS_SNAPSHOT_ESCAPED_TEXT_BUDGET: usize = 12 * 1024 * 1024;

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
    pub first_display_sequence: u64,
    pub event_count: usize,
    pub observed_packet_count: usize,
    pub packets_with_hits: usize,
    pub retained_packet_count: usize,
    pub queued_event_count: usize,
    pub truncated_packet_count: usize,
    pub omitted_text_bytes: usize,
    pub omitted_declared_id_count: usize,
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
    pub omitted_text_bytes: usize,
    pub omitted_declared_id_count: usize,
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
    let first_display_sequence = state
        .packets_generation
        .saturating_sub(PACKETS_DISPLAY_LIMIT.saturating_sub(1) as u64)
        .max(retained_first_sequence);
    let retained = state.packets.iter().skip(skip.min(state.packets.len()));
    let mut packets = Vec::with_capacity(retained.len().min(PACKETS_DISPLAY_LIMIT));
    let mut escaped_text_budget = PACKETS_SNAPSHOT_ESCAPED_TEXT_BUDGET;
    let mut truncated_packet_count = 0_usize;
    let mut omitted_text_bytes = 0_usize;
    let mut omitted_declared_id_count = 0_usize;
    for (offset, packet) in retained.enumerate() {
        let packet = PacketProjection::from_packet(
            first_sequence.saturating_add(offset as u64),
            packet,
            &mut escaped_text_budget,
        );
        if packet.omitted_text_bytes > 0 || packet.omitted_declared_id_count > 0 {
            truncated_packet_count = truncated_packet_count.saturating_add(1);
        }
        omitted_text_bytes = omitted_text_bytes.saturating_add(packet.omitted_text_bytes);
        omitted_declared_id_count =
            omitted_declared_id_count.saturating_add(packet.omitted_declared_id_count);
        packets.push(packet);
    }
    PacketsProjection {
        generation: revision.generation,
        session_generation: revision.session_generation,
        packet_generation: revision.packet_generation,
        first_display_sequence,
        event_count: state.hits.len(),
        observed_packet_count: state.packet_count,
        packets_with_hits: state.packets_with_hits,
        retained_packet_count: state.packets.len(),
        queued_event_count,
        truncated_packet_count,
        omitted_text_bytes,
        omitted_declared_id_count,
        packets,
    }
}

impl PacketProjection {
    fn from_packet(sequence: u64, packet: &PacketDebug, escaped_text_budget: &mut usize) -> Self {
        let (source, source_omitted) = bounded_packet_text(
            &packet.source,
            PACKETS_MAX_SOURCE_BYTES,
            escaped_text_budget,
        );
        let (destination, destination_omitted) = bounded_packet_text(
            &packet.destination,
            PACKETS_MAX_DESTINATION_BYTES,
            escaped_text_budget,
        );
        let (direction, direction_omitted) = bounded_packet_text(
            &packet.direction,
            PACKETS_MAX_DIRECTION_BYTES,
            escaped_text_budget,
        );
        let (note, note_omitted) =
            bounded_packet_text(&packet.note, PACKETS_MAX_NOTE_BYTES, escaped_text_budget);
        let (decoded_text, decoded_text_omitted) = bounded_packet_text(
            &packet.decoded_text,
            PACKETS_MAX_DECODED_TEXT_BYTES,
            escaped_text_budget,
        );
        let omitted_text_bytes = source_omitted
            .saturating_add(destination_omitted)
            .saturating_add(direction_omitted)
            .saturating_add(note_omitted)
            .saturating_add(decoded_text_omitted);
        let omitted_declared_id_count = packet
            .declared_ids
            .len()
            .saturating_sub(PACKETS_MAX_DECLARED_IDS);
        Self {
            sequence,
            timestamp: packet.timestamp,
            source,
            destination,
            direction,
            payload_len: packet.payload_len,
            declared_ids: packet
                .declared_ids
                .iter()
                .take(PACKETS_MAX_DECLARED_IDS)
                .copied()
                .collect(),
            parsed_hits: packet.parsed_hits,
            note,
            decoded_text,
            omitted_text_bytes,
            omitted_declared_id_count,
        }
    }
}

fn bounded_packet_text(
    value: &str,
    max_utf8_bytes: usize,
    escaped_text_budget: &mut usize,
) -> (String, usize) {
    let mut end = 0_usize;
    let mut escaped_bytes = 0_usize;
    for (offset, character) in value.char_indices() {
        let next_end = offset.saturating_add(character.len_utf8());
        let next_escaped_bytes = escaped_bytes.saturating_add(json_escaped_char_bytes(character));
        if next_end > max_utf8_bytes || next_escaped_bytes > *escaped_text_budget {
            break;
        }
        end = next_end;
        escaped_bytes = next_escaped_bytes;
    }
    *escaped_text_budget = escaped_text_budget.saturating_sub(escaped_bytes);
    (value[..end].to_owned(), value.len().saturating_sub(end))
}

/// Conservative upper bound for serde_json string-content encoding. JSON may
/// use two-byte short escapes for common controls; counting every control as a
/// six-byte `\u00XX` escape keeps the cumulative source budget fail-closed.
const fn json_escaped_char_bytes(character: char) -> usize {
    match character {
        '"' | '\\' => 2,
        '\u{0000}'..='\u{001f}' => 6,
        _ => character.len_utf8(),
    }
}

#[cfg(test)]
fn projected_escaped_text_bytes(projection: &PacketsProjection) -> usize {
    projection
        .packets
        .iter()
        .flat_map(|packet| {
            [
                packet.source.as_str(),
                packet.destination.as_str(),
                packet.direction.as_str(),
                packet.note.as_str(),
                packet.decoded_text.as_str(),
            ]
        })
        .flat_map(str::chars)
        .map(json_escaped_char_bytes)
        .sum()
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
        assert_eq!(projection.first_display_sequence, 4);
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
        assert_eq!(projection.first_display_sequence, 1);
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

    #[test]
    fn projection_bounds_every_utf8_field_ids_and_cumulative_json_escapes() {
        let worst_escape_text = "\0".repeat(4_096);
        let mut state = CombatState::default();
        for index in 0..PACKETS_DISPLAY_LIMIT {
            assert!(state.push_packet(PacketDebug {
                timestamp: index as f64,
                source: "\0".repeat(PACKETS_MAX_SOURCE_BYTES + 1),
                destination: "\0".repeat(PACKETS_MAX_DESTINATION_BYTES + 1),
                direction: "\0".repeat(PACKETS_MAX_DIRECTION_BYTES + 1),
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

        let projection = project_recent_packets(&state, revision(&state), usize::MAX);

        assert_eq!(projection.packets.len(), PACKETS_DISPLAY_LIMIT);
        assert_eq!(projection.packets[0].sequence, 1);
        assert_eq!(
            projection.packets.last().map(|packet| packet.sequence),
            Some(PACKETS_DISPLAY_LIMIT as u64)
        );
        assert_eq!(projection.truncated_packet_count, PACKETS_DISPLAY_LIMIT);
        assert!(projection.omitted_text_bytes > 0);
        assert_eq!(
            projection.omitted_declared_id_count,
            PACKETS_DISPLAY_LIMIT * 44
        );
        assert!(projected_escaped_text_bytes(&projection) <= PACKETS_SNAPSHOT_ESCAPED_TEXT_BUDGET);
        for packet in &projection.packets {
            assert!(packet.source.len() <= PACKETS_MAX_SOURCE_BYTES);
            assert!(packet.destination.len() <= PACKETS_MAX_DESTINATION_BYTES);
            assert!(packet.direction.len() <= PACKETS_MAX_DIRECTION_BYTES);
            assert!(packet.note.len() <= PACKETS_MAX_NOTE_BYTES);
            assert!(packet.decoded_text.len() <= PACKETS_MAX_DECODED_TEXT_BYTES);
            assert!(packet.declared_ids.len() <= PACKETS_MAX_DECLARED_IDS);
        }
        assert_eq!(
            projection.omitted_text_bytes,
            projection
                .packets
                .iter()
                .map(|packet| packet.omitted_text_bytes)
                .sum::<usize>()
        );
    }

    #[test]
    fn utf8_prefixes_remain_valid_and_json_escape_estimate_is_conservative() {
        let value = "界".repeat(200);
        let mut budget = usize::MAX;
        let (bounded, omitted) = bounded_packet_text(&value, 512, &mut budget);
        assert_eq!(bounded.len(), 510);
        assert_eq!(omitted, value.len() - bounded.len());

        let sample = (0_u8..=31)
            .map(char::from)
            .chain(['"', '\\', '\u{2028}', '界'])
            .collect::<String>();
        let estimate = sample.chars().map(json_escaped_char_bytes).sum::<usize>();
        let encoded = serde_json::to_vec(&sample).expect("sample serializes");
        assert!(encoded.len().saturating_sub(2) <= estimate);
    }
}
