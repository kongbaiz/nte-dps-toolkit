const HANDLER_PREFIX_BITS: usize = 3;
const SEQUENCED_SIGNATURE: u8 = 3;
const HANDSHAKE_SIGNATURE: u8 = 7;
const SEQUENCED_HEADER_BITS: usize = 72;

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
    if signature != SEQUENCED_SIGNATURE || data_bit_len < SEQUENCED_HEADER_BITS {
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
    const HEADER_BITS: usize = 48;
    if packet.mode != 0 || packet.payload_bit_len < HEADER_BITS + 1 {
        return None;
    }

    let prefix = read_bits_le(&packet.payload, 0, 13)? as u16;
    let sequence = read_bits_le(&packet.payload, 13, 14)? as u16;
    let descriptor = read_bits_le(&packet.payload, 27, 8)? as u8;
    let data_bit_len = read_bits_le(&packet.payload, 35, 13)? as usize;
    if packet.payload_bit_len != HEADER_BITS + data_bit_len + 1
        || read_bits_le(&packet.payload, HEADER_BITS + data_bit_len, 1)? != 1
    {
        return None;
    }

    Some(SingleBunch {
        prefix,
        sequence,
        descriptor,
        data_bit_len,
        data: extract_bits(&packet.payload, HEADER_BITS, data_bit_len)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ack_only_packet_from_real_capture() {
        let packet = hex::decode("18a890a7650000008001").unwrap();
        let parsed = parse_transport_packet(&packet).unwrap();
        let TransportPacket::Sequenced(parsed) = parsed else {
            panic!("expected sequenced packet");
        };

        assert_eq!(parsed.handler_prefix, 0);
        assert_eq!(parsed.mode, 0);
        assert_eq!(parsed.header_flags, 0);
        assert_eq!(parsed.acknowledged_packet_id, 9258);
        assert_eq!(parsed.packet_id, 9639);
        assert_eq!(parsed.acknowledgment_history, 1);
        assert_eq!(parsed.packet_flags, 2);
        assert_eq!(parsed.payload_bit_len, 0);
        assert!(parsed.payload.is_empty());
    }

    #[test]
    fn preserves_non_byte_aligned_payload() {
        let packet = hex::decode(concat!(
            "18a49635e4070000",
            "002298056406ee01002265050e2206704e0803856884153870",
            "03"
        ))
        .unwrap();
        let parsed = parse_transport_packet(&packet).unwrap();
        let TransportPacket::Sequenced(parsed) = parsed else {
            panic!("expected sequenced packet");
        };

        assert_eq!(parsed.mode, 0);
        assert_eq!(parsed.acknowledged_packet_id, 9641);
        assert_eq!(parsed.packet_id, 9269);
        assert_eq!(parsed.acknowledgment_history, 31);
        assert_eq!(parsed.packet_flags, 0);
        assert_eq!(parsed.payload_bit_len, 193);
        assert_eq!(
            parsed.payload,
            hex::decode("2298056406ee01002265050e2206704e080385688415387001").unwrap()
        );
    }

    #[test]
    fn exposes_alternate_packet_mode_without_guessing_its_semantics() {
        let packet = hex::decode(concat!("586091ade5ffffffffff1f00", "0102030401")).unwrap();
        let parsed = parse_transport_packet(&packet).unwrap();
        let TransportPacket::Sequenced(parsed) = parsed else {
            panic!("expected sequenced packet");
        };

        assert_eq!(parsed.mode, 1);
    }

    #[test]
    fn classifies_stateless_handshake_packet() {
        let packet = hex::decode(concat!(
            "b801028000782af91e0000000000000000000000000000000000000000000000",
            "0000000000009b6ce1c32628a5ee3f"
        ))
        .unwrap();

        assert!(matches!(
            parse_transport_packet(&packet),
            Some(TransportPacket::StatelessHandshake {
                handler_prefix: 0,
                ..
            })
        ));
    }

    #[test]
    fn rejects_packet_without_termination_marker() {
        assert!(parse_transport_packet(&[0_u8; 10]).is_none());
    }

    #[test]
    fn parses_single_bunch_bit_layout() {
        let data = [0x12, 0x34, 0x01];
        let data_bit_len = 18;
        let prefix = 0x102_u64;
        let sequence = 8236_u64;
        let descriptor = 0xcc_u64;
        let mut bits =
            prefix | (sequence << 13) | (descriptor << 27) | ((data_bit_len as u64) << 35);
        bits |= u64::from(data[0]) << 48;
        bits |= u64::from(data[1]) << 56;
        let mut payload = bits.to_le_bytes().to_vec();
        payload.push(data[2] & 0x03);
        let marker_offset = 48 + data_bit_len;
        payload[marker_offset / 8] |= 1 << (marker_offset % 8);
        let packet = SequencedPacket {
            handler_prefix: 0,
            mode: 0,
            header_flags: 0,
            acknowledged_packet_id: 0,
            packet_id: 0,
            acknowledgment_history: 0,
            packet_flags: 0,
            payload_bit_len: marker_offset + 1,
            payload,
        };

        let bunch = parse_single_bunch(&packet).unwrap();
        assert_eq!(bunch.prefix, 0x102);
        assert_eq!(bunch.sequence, 8236);
        assert_eq!(bunch.descriptor, 0xcc);
        assert_eq!(bunch.data_bit_len, 18);
        assert_eq!(bunch.data, data);
    }
}
