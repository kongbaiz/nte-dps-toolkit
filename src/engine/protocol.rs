use std::collections::HashSet;

const HANDLER_PREFIX_BITS: usize = 3;
const HANDSHAKE_SIGNATURE: u8 = 7;
const SEQUENCED_HEADER_BITS: usize = 72;
const BUNCH_HEADER_BITS: usize = 48;
const INVENTORY_BUNCH_DESCRIPTOR: u8 = 0xcc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequencedPacket {
    pub handler_prefix: u8,
    pub mode: u8,
    pub header_flags: u8,
    pub acknowledged_packet_id: u16,
    pub packet_id: u16,
    pub acknowledgment_history: u32,
    pub packet_flags: u8,
    pub payload_bit_len: usize,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SingleBunch {
    pub prefix: u16,
    pub sequence: u16,
    pub descriptor: u8,
    pub partial_flags: u8,
    pub data_bit_len: usize,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportPacket {
    StatelessHandshake {
        handler_prefix: u8,
        payload_bit_len: usize,
    },
    Sequenced(SequencedPacket),
}

fn read_bits_le(data: &[u8], bit_offset: usize, bit_count: usize) -> Option<u64> {
    if bit_count > 64 || bit_offset.checked_add(bit_count)? > data.len() * 8 {
        return None;
    }
    let mut value = 0_u64;
    for index in 0..bit_count {
        let source_bit = bit_offset + index;
        let bit = (data[source_bit / 8] >> (source_bit % 8)) & 1;
        value |= u64::from(bit) << index;
    }
    Some(value)
}

fn packet_data_bit_len(data: &[u8]) -> Option<usize> {
    let last = *data.last()?;
    if last == 0 {
        return None;
    }
    let termination_bit = 7 - last.leading_zeros() as usize;
    Some((data.len() - 1) * 8 + termination_bit)
}

fn extract_bits(data: &[u8], bit_offset: usize, bit_len: usize) -> Option<Vec<u8>> {
    if bit_len == 0 {
        return Some(Vec::new());
    }
    let byte_len = bit_len.div_ceil(8);
    let mut output = Vec::with_capacity(byte_len);
    for output_index in 0..byte_len {
        let remaining = bit_len - output_index * 8;
        let width = remaining.min(8);
        output.push(read_bits_le(data, bit_offset + output_index * 8, width)? as u8);
    }
    let trailing_bits = bit_len % 8;
    if trailing_bits != 0 {
        let mask = (1_u8 << trailing_bits) - 1;
        if let Some(last) = output.last_mut() {
            *last &= mask;
        }
    }
    Some(output)
}

pub fn parse_transport_packet(data: &[u8]) -> Option<TransportPacket> {
    let data_bit_len = packet_data_bit_len(data)?;
    let handler_prefix = read_bits_le(data, 0, HANDLER_PREFIX_BITS)? as u8;
    let signature = read_bits_le(data, 3, 3)? as u8;

    if signature == HANDSHAKE_SIGNATURE {
        return Some(TransportPacket::StatelessHandshake {
            handler_prefix,
            payload_bit_len: data_bit_len.saturating_sub(6),
        });
    }
    // The six PacketHandler prefix bits vary per connection. Handshake packets
    // retain their explicit marker; every other sufficiently long game packet
    // uses the sequenced header validated by its downstream payload parser.
    if data_bit_len < SEQUENCED_HEADER_BITS {
        return None;
    }

    let payload_bit_len = data_bit_len - SEQUENCED_HEADER_BITS;
    Some(TransportPacket::Sequenced(SequencedPacket {
        handler_prefix,
        mode: read_bits_le(data, 6, 2)? as u8,
        header_flags: read_bits_le(data, 8, 2)? as u8,
        acknowledged_packet_id: read_bits_le(data, 10, 14)? as u16,
        packet_id: read_bits_le(data, 24, 14)? as u16,
        acknowledgment_history: read_bits_le(data, 38, 32)? as u32,
        packet_flags: read_bits_le(data, 70, 2)? as u8,
        payload_bit_len,
        payload: extract_bits(data, SEQUENCED_HEADER_BITS, payload_bit_len)?,
    }))
}

pub fn parse_single_bunch(packet: &SequencedPacket) -> Option<SingleBunch> {
    if packet.mode != 0 || packet.payload_bit_len < BUNCH_HEADER_BITS + 1 {
        return None;
    }

    let prefix = read_bits_le(&packet.payload, 0, 13)? as u16;
    let sequence = read_bits_le(&packet.payload, 13, 10)? as u16;
    let bunch_flags = read_bits_le(&packet.payload, 23, 12)? as u16;
    let descriptor = (bunch_flags >> 4) as u8;
    let partial_flags = (bunch_flags & 0x0f) as u8;
    let data_bit_len = read_bits_le(&packet.payload, 35, 13)? as usize;
    if packet.payload_bit_len != BUNCH_HEADER_BITS + data_bit_len + 1
        || read_bits_le(&packet.payload, BUNCH_HEADER_BITS + data_bit_len, 1)? != 1
    {
        return None;
    }

    Some(SingleBunch {
        prefix,
        sequence,
        descriptor,
        partial_flags,
        data_bit_len,
        data: extract_bits(&packet.payload, BUNCH_HEADER_BITS, data_bit_len)?,
    })
}

fn is_inventory_partial_flags(partial_flags: u8) -> bool {
    matches!(partial_flags, 0x08 | 0x09 | 0x0c | 0x0d)
}

fn parse_inventory_bunch_at(
    packet: &SequencedPacket,
    bit_offset: usize,
    require_exact_tail: bool,
) -> Option<SingleBunch> {
    let header_end = bit_offset.checked_add(BUNCH_HEADER_BITS)?;
    if header_end > packet.payload_bit_len {
        return None;
    }

    let bunch_flags = read_bits_le(&packet.payload, bit_offset + 23, 12)? as u16;
    let descriptor = (bunch_flags >> 4) as u8;
    let partial_flags = (bunch_flags & 0x0f) as u8;
    if descriptor != INVENTORY_BUNCH_DESCRIPTOR || !is_inventory_partial_flags(partial_flags) {
        return None;
    }

    let data_bit_len = read_bits_le(&packet.payload, bit_offset + 35, 13)? as usize;
    if data_bit_len == 0 {
        return None;
    }
    let prefix = read_bits_le(&packet.payload, bit_offset, 13)? as u16;
    let sequence = read_bits_le(&packet.payload, bit_offset + 13, 10)? as u16;
    let data_end = header_end.checked_add(data_bit_len)?;
    if data_end > packet.payload_bit_len {
        return None;
    }
    if require_exact_tail {
        let terminated_end = data_end.checked_add(1)?;
        if terminated_end != packet.payload_bit_len
            || read_bits_le(&packet.payload, data_end, 1)? != 1
        {
            return None;
        }
    }

    Some(SingleBunch {
        prefix,
        sequence,
        descriptor,
        partial_flags,
        data_bit_len,
        data: extract_bits(&packet.payload, header_end, data_bit_len)?,
    })
}

/// Finds inventory partial bunches in one sequenced packet.
///
/// Exact-tail bunches are high-confidence candidates and make their channel available to the
/// second pass over the same packet. Callers should retain the returned channels and pass them in
/// `known_channels` for later packets. The function deliberately performs no cross-packet state or
/// partial-chain reassembly.
pub fn parse_inventory_bunches(
    packet: &SequencedPacket,
    known_channels: &[u16],
) -> Vec<SingleBunch> {
    if packet.mode != 0 || packet.payload_bit_len < BUNCH_HEADER_BITS + 1 {
        return Vec::new();
    }
    let Some(last_start) = packet.payload_bit_len.checked_sub(BUNCH_HEADER_BITS) else {
        return Vec::new();
    };

    let mut channels = known_channels.iter().copied().collect::<HashSet<_>>();
    let mut exact_tail = Vec::new();
    for bit_offset in 0..=last_start {
        if let Some(bunch) = parse_inventory_bunch_at(packet, bit_offset, true) {
            channels.insert(bunch.prefix);
            exact_tail.push((bit_offset, bunch));
        }
    }
    if channels.is_empty() {
        return Vec::new();
    }

    let exact_keys = exact_tail
        .iter()
        .map(|(_, bunch)| (bunch.prefix, bunch.sequence))
        .collect::<HashSet<_>>();
    let mut candidates = exact_tail;
    for bit_offset in 0..=last_start {
        let Some(bunch) = parse_inventory_bunch_at(packet, bit_offset, false) else {
            continue;
        };
        let key = (bunch.prefix, bunch.sequence);
        if channels.contains(&bunch.prefix) && !exact_keys.contains(&key) {
            candidates.push((bit_offset, bunch));
        }
    }

    candidates.sort_by_key(|(bit_offset, _)| *bit_offset);
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter_map(|(_, bunch)| seen.insert((bunch.prefix, bunch.sequence)).then_some(bunch))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_bits(data: &mut [u8], bit_offset: usize, bit_count: usize, value: u64) {
        for index in 0..bit_count {
            let target = bit_offset + index;
            data[target / 8] |= (((value >> index) & 1) as u8) << (target % 8);
        }
    }

    fn single_bunch_packet(mode: u8, declared_len: usize, actual_data: &[u8]) -> SequencedPacket {
        let actual_data_bits = actual_data.len() * 8;
        let payload_bit_len = BUNCH_HEADER_BITS + actual_data_bits + 1;
        let mut payload = vec![0_u8; payload_bit_len.div_ceil(8)];
        write_bits(&mut payload, 0, 13, 4122);
        write_bits(&mut payload, 13, 10, 87);
        write_bits(&mut payload, 23, 12, 0xcc9);
        write_bits(&mut payload, 35, 13, declared_len as u64);
        for (index, byte) in actual_data.iter().copied().enumerate() {
            write_bits(
                &mut payload,
                BUNCH_HEADER_BITS + index * 8,
                8,
                u64::from(byte),
            );
        }
        write_bits(&mut payload, BUNCH_HEADER_BITS + actual_data_bits, 1, 1);
        SequencedPacket {
            handler_prefix: 0,
            mode,
            header_flags: 0,
            acknowledged_packet_id: 0,
            packet_id: 0,
            acknowledgment_history: 0,
            packet_flags: 0,
            payload_bit_len,
            payload,
        }
    }

    fn write_bunch(
        payload: &mut [u8],
        bit_offset: usize,
        prefix: u16,
        sequence: u16,
        descriptor: u8,
        partial_flags: u8,
        data: &[u8],
        data_bit_len: usize,
    ) -> usize {
        assert!(data_bit_len <= data.len() * 8);
        write_bits(payload, bit_offset, 13, u64::from(prefix));
        write_bits(payload, bit_offset + 13, 10, u64::from(sequence));
        write_bits(
            payload,
            bit_offset + 23,
            12,
            u64::from((u16::from(descriptor) << 4) | u16::from(partial_flags)),
        );
        write_bits(payload, bit_offset + 35, 13, data_bit_len as u64);
        for index in 0..data_bit_len {
            let value = (data[index / 8] >> (index % 8)) & 1;
            write_bits(
                payload,
                bit_offset + BUNCH_HEADER_BITS + index,
                1,
                u64::from(value),
            );
        }
        bit_offset + BUNCH_HEADER_BITS + data_bit_len
    }

    fn sequenced_packet(payload: Vec<u8>, payload_bit_len: usize) -> SequencedPacket {
        SequencedPacket {
            handler_prefix: 0,
            mode: 0,
            header_flags: 0,
            acknowledged_packet_id: 0,
            packet_id: 0,
            acknowledgment_history: 0,
            packet_flags: 0,
            payload_bit_len,
            payload,
        }
    }

    fn transport_packet(handler_prefix: u8, signature: u8) -> Vec<u8> {
        let mut packet = vec![0_u8; 10];
        write_bits(&mut packet, 0, 3, u64::from(handler_prefix));
        write_bits(&mut packet, 3, 3, u64::from(signature));
        write_bits(&mut packet, SEQUENCED_HEADER_BITS, 1, 1);
        packet
    }

    #[test]
    fn accepts_connection_specific_sequenced_prefix_bits() {
        for (handler_prefix, signature) in [(0, 0), (4, 0), (0, 2), (0, 3)] {
            let data = transport_packet(handler_prefix, signature);

            let packet = parse_transport_packet(&data)
                .expect("observed sequenced prefix should parse as transport packet");

            assert!(matches!(packet, TransportPacket::Sequenced(_)));
        }
    }

    #[test]
    fn preserves_explicit_handshake_marker() {
        let data = transport_packet(0, HANDSHAKE_SIGNATURE);

        assert!(matches!(
            parse_transport_packet(&data),
            Some(TransportPacket::StatelessHandshake { .. })
        ));
    }

    #[test]
    fn parses_channel_sequence_and_partial_flags_separately() {
        let packet = single_bunch_packet(0, 16, &[0x5a, 0xa5]);

        let bunch = parse_single_bunch(&packet).expect("valid single bunch should parse");

        assert_eq!(bunch.prefix, 4122);
        assert_eq!(bunch.sequence, 87);
        assert_eq!(bunch.descriptor, 0xcc);
        assert_eq!(bunch.partial_flags, 0x09);
        assert_eq!(bunch.data_bit_len, 16);
        assert_eq!(bunch.data, [0x5a, 0xa5]);
    }

    #[test]
    fn rejects_declared_length_that_crosses_the_packet_boundary() {
        let packet = single_bunch_packet(0, 17, &[0x5a, 0xa5]);

        assert!(parse_single_bunch(&packet).is_none());
    }

    #[test]
    fn rejects_nonzero_transport_mode() {
        let packet = single_bunch_packet(1, 16, &[0x5a, 0xa5]);

        assert!(parse_single_bunch(&packet).is_none());
    }

    #[test]
    fn finds_non_byte_aligned_inventory_bunch_after_packet_info() {
        // The sequenced payload starts at packet bit 72, so this reproduces a bunch at bit 91.
        let bunch_offset = 19;
        let data = [0x5a, 0x15];
        let data_bit_len = 13;
        let payload_bit_len = bunch_offset + BUNCH_HEADER_BITS + data_bit_len + 1;
        let mut payload = vec![0_u8; payload_bit_len.div_ceil(8)];
        write_bits(&mut payload, 0, bunch_offset, 0x5a55);
        let data_end = write_bunch(
            &mut payload,
            bunch_offset,
            4122,
            550,
            INVENTORY_BUNCH_DESCRIPTOR,
            0x09,
            &data,
            data_bit_len,
        );
        write_bits(&mut payload, data_end, 1, 1);
        let packet = sequenced_packet(payload, payload_bit_len);

        assert!(parse_single_bunch(&packet).is_none());
        assert_eq!(
            parse_inventory_bunches(&packet, &[]),
            vec![SingleBunch {
                prefix: 4122,
                sequence: 550,
                descriptor: INVENTORY_BUNCH_DESCRIPTOR,
                partial_flags: 0x09,
                data_bit_len,
                data: data.to_vec(),
            }]
        );
    }

    #[test]
    fn exact_tail_channel_discovers_earlier_bunches_in_the_same_packet() {
        let first_data = [0x33, 0xcc];
        let second_data = [0xa5, 0x03];
        let first_offset = 0;
        let second_offset = first_offset + BUNCH_HEADER_BITS + 16;
        let payload_bit_len = second_offset + BUNCH_HEADER_BITS + 11 + 1;
        let mut payload = vec![0_u8; payload_bit_len.div_ceil(8)];
        write_bunch(
            &mut payload,
            first_offset,
            4122,
            100,
            INVENTORY_BUNCH_DESCRIPTOR,
            0x09,
            &first_data,
            16,
        );
        let data_end = write_bunch(
            &mut payload,
            second_offset,
            4122,
            101,
            INVENTORY_BUNCH_DESCRIPTOR,
            0x0c,
            &second_data,
            11,
        );
        write_bits(&mut payload, data_end, 1, 1);
        let packet = sequenced_packet(payload, payload_bit_len);

        let bunches = parse_inventory_bunches(&packet, &[]);

        assert_eq!(bunches.len(), 2);
        assert_eq!(bunches[0].sequence, 100);
        assert_eq!(bunches[0].data, first_data);
        assert_eq!(bunches[1].sequence, 101);
        assert_eq!(bunches[1].data_bit_len, 11);
        assert_eq!(bunches[1].data, second_data);
    }

    #[test]
    fn known_channel_finds_embedded_bunch_without_an_exact_tail() {
        let bunch_offset = 7;
        let payload_bit_len = bunch_offset + BUNCH_HEADER_BITS + 8 + 9;
        let mut payload = vec![0_u8; payload_bit_len.div_ceil(8)];
        write_bunch(
            &mut payload,
            bunch_offset,
            4122,
            87,
            INVENTORY_BUNCH_DESCRIPTOR,
            0x08,
            &[0x5a],
            8,
        );
        let packet = sequenced_packet(payload, payload_bit_len);

        assert!(parse_inventory_bunches(&packet, &[]).is_empty());
        assert_eq!(parse_inventory_bunches(&packet, &[4122]).len(), 1);
    }

    #[test]
    fn rejects_invalid_inventory_flags_and_descriptor() {
        for (descriptor, partial_flags) in [(INVENTORY_BUNCH_DESCRIPTOR, 0x0a), (0xcb, 0x09)] {
            let payload_bit_len = BUNCH_HEADER_BITS + 8 + 1;
            let mut payload = vec![0_u8; payload_bit_len.div_ceil(8)];
            let data_end = write_bunch(
                &mut payload,
                0,
                4122,
                87,
                descriptor,
                partial_flags,
                &[0x5a],
                8,
            );
            write_bits(&mut payload, data_end, 1, 1);
            let packet = sequenced_packet(payload, payload_bit_len);

            assert!(parse_inventory_bunches(&packet, &[4122]).is_empty());
        }
    }

    #[test]
    fn rejects_inventory_bunch_length_past_payload_boundary() {
        let payload_bit_len = BUNCH_HEADER_BITS + 8;
        let mut payload = vec![0_u8; payload_bit_len.div_ceil(8)];
        write_bits(&mut payload, 0, 13, 4122);
        write_bits(&mut payload, 13, 10, 87);
        write_bits(&mut payload, 23, 12, 0xcc9);
        write_bits(&mut payload, 35, 13, 100);
        let packet = sequenced_packet(payload, payload_bit_len);

        assert!(parse_inventory_bunches(&packet, &[4122]).is_empty());
    }
}
