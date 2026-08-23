use std::collections::{HashMap, HashSet, VecDeque};

const HANDLER_PREFIX_BITS: usize = 3;
const HANDSHAKE_SIGNATURE: u8 = 7;
const SEQUENCED_HEADER_BITS: usize = 72;
const BUNCH_HEADER_BITS: usize = 48;
const INVENTORY_BUNCH_DESCRIPTOR: u8 = 0xcc;
const MAX_BUNCHES_PER_PACKET: usize = 64;
const BUNCH_SEQUENCE_MASK: u16 = 0x03ff;

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
pub struct LocatedBunch {
    pub bit_offset: usize,
    pub bunch: SingleBunch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BunchPacket {
    /// Bits before the first decoded Bunch. These are retained as packet-level information.
    pub packet_info_bit_len: usize,
    pub bunches: Vec<LocatedBunch>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BunchParseError {
    UnsupportedMode,
    PayloadTooShort,
    MissingTerminator,
    NoTailBunch,
    AmbiguousTail,
    AmbiguousPredecessor,
    TooManyBunches,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReassembledBunch {
    pub channel: u16,
    pub descriptor: u8,
    pub first_sequence: u16,
    pub last_sequence: u16,
    pub fragment_count: usize,
    pub data_bit_len: usize,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug)]
struct StoredBunch {
    bunch: SingleBunch,
    packet_order: i64,
}

/// Connection-local, bounded reassembly for the observed reliable Bunch fragment flags.
///
/// One capture decoder owns each instance. Fragments are keyed by channel and 10-bit reliable
/// sequence. Capacity eviction drops the oldest incomplete fragment; malformed, oversized, stale,
/// or cross-descriptor chains fail closed instead of returning partially joined bytes.
pub struct BunchReassembler {
    known_channels: HashSet<u16>,
    verified_partial_profiles: HashSet<(u16, u8)>,
    fragments: HashMap<(u16, u16), StoredBunch>,
    fragment_order: VecDeque<(u16, u16)>,
    latest_packet_order: Option<i64>,
    max_fragments: usize,
    max_stream_bits: usize,
    max_packet_span: i64,
}

impl BunchReassembler {
    pub fn new(max_fragments: usize, max_stream_bits: usize, max_packet_span: i64) -> Self {
        Self {
            known_channels: HashSet::new(),
            verified_partial_profiles: HashSet::new(),
            fragments: HashMap::new(),
            fragment_order: VecDeque::new(),
            latest_packet_order: None,
            max_fragments: max_fragments.max(1),
            max_stream_bits,
            max_packet_span: max_packet_span.max(0),
        }
    }

    pub fn observe_packet(
        &mut self,
        packet_id: u16,
        bunches: impl IntoIterator<Item = SingleBunch>,
    ) -> Vec<ReassembledBunch> {
        let packet_order = unwrap_packet_id(packet_id, self.latest_packet_order);
        if self
            .latest_packet_order
            .is_none_or(|latest| packet_order > latest)
        {
            self.latest_packet_order = Some(packet_order);
        }

        let mut completed = Vec::new();
        for bunch in bunches {
            let channel = reliable_bunch_channel(bunch.prefix);
            self.known_channels.insert(channel);
            match bunch.partial_flags {
                // Observed complete, non-fragmented Bunches and a one-fragment partial stream.
                0x04 | 0x05 | 0x0d => {
                    completed.push(ReassembledBunch {
                        channel,
                        descriptor: bunch.descriptor,
                        first_sequence: bunch.sequence,
                        last_sequence: bunch.sequence,
                        fragment_count: 1,
                        data_bit_len: bunch.data_bit_len,
                        data: bunch.data,
                    });
                }
                // Initial, continuation, and final partial fragments.
                0x08 | 0x09 | 0x0c => {
                    self.insert_fragment(packet_order, channel, bunch);
                    completed.extend(self.take_completed_streams());
                }
                _ => {}
            }
        }
        self.drop_stale_fragments();
        completed
    }

    pub fn known_channels(&self) -> impl Iterator<Item = u16> + '_ {
        self.known_channels.iter().copied()
    }

    pub fn expected_continuations(&self) -> Vec<(u16, u16, u8)> {
        let mut expected = self
            .fragments
            .iter()
            .filter_map(|(&(channel, sequence), stored)| {
                matches!(stored.bunch.partial_flags, 0x08 | 0x09).then_some((
                    channel,
                    (sequence + 1) & BUNCH_SEQUENCE_MASK,
                    stored.bunch.descriptor,
                ))
            })
            .collect::<Vec<_>>();
        expected.sort_unstable();
        expected.dedup();
        expected
    }

    pub fn verified_partial_profiles(&self) -> Vec<(u16, u8)> {
        let mut profiles = self
            .verified_partial_profiles
            .iter()
            .copied()
            .collect::<Vec<_>>();
        profiles.sort_unstable();
        profiles
    }

    fn insert_fragment(&mut self, packet_order: i64, channel: u16, bunch: SingleBunch) {
        let key = (channel, bunch.sequence);
        if let Some(stored) = self.fragments.get(&key) {
            if stored.bunch == bunch {
                return;
            }
            if packet_order <= stored.packet_order {
                return;
            }
        }
        if self.fragments.remove(&key).is_some() {
            self.fragment_order.retain(|stored| *stored != key);
        }
        while self.fragments.len() >= self.max_fragments {
            let Some(oldest) = self.fragment_order.pop_front() else {
                break;
            };
            self.fragments.remove(&oldest);
        }
        self.fragment_order.push_back(key);
        self.fragments.insert(
            key,
            StoredBunch {
                bunch,
                packet_order,
            },
        );
    }

    fn take_completed_streams(&mut self) -> Vec<ReassembledBunch> {
        let mut starts = self
            .fragments
            .iter()
            .filter_map(|(key, stored)| (stored.bunch.partial_flags == 0x09).then_some(*key))
            .collect::<Vec<_>>();
        starts.sort_unstable();

        let mut completed = Vec::new();
        let mut consumed = HashSet::new();
        for start @ (channel, first_sequence) in starts {
            let Some(initial) = self.fragments.get(&start) else {
                continue;
            };
            let descriptor = initial.bunch.descriptor;
            let mut data = Vec::new();
            let mut data_bit_len = 0;
            let mut sequence = first_sequence;
            let mut last_sequence = first_sequence;
            let mut fragment_count = 0;
            let mut min_packet_order = None;
            let mut max_packet_order = None;
            let mut chain_keys = Vec::new();
            let mut is_complete = false;

            for index in 0..=usize::from(BUNCH_SEQUENCE_MASK) {
                let key = (channel, sequence);
                let Some(stored) = self.fragments.get(&key) else {
                    break;
                };
                let next_min = min_packet_order.map_or(stored.packet_order, |current: i64| {
                    current.min(stored.packet_order)
                });
                let next_max = max_packet_order.map_or(stored.packet_order, |current: i64| {
                    current.max(stored.packet_order)
                });
                if next_max - next_min > self.max_packet_span {
                    break;
                }
                min_packet_order = Some(next_min);
                max_packet_order = Some(next_max);

                let fragment = &stored.bunch;
                let valid_flag = if index == 0 {
                    fragment.partial_flags == 0x09
                } else {
                    matches!(fragment.partial_flags, 0x08 | 0x0c)
                };
                if !valid_flag
                    || fragment.descriptor != descriptor
                    || append_bounded_bits(
                        &mut data,
                        &mut data_bit_len,
                        &fragment.data,
                        fragment.data_bit_len,
                        self.max_stream_bits,
                    )
                    .is_none()
                {
                    break;
                }
                fragment_count += 1;
                last_sequence = sequence;
                chain_keys.push(key);
                if fragment.partial_flags == 0x0c {
                    is_complete = true;
                    break;
                }
                sequence = (sequence + 1) & BUNCH_SEQUENCE_MASK;
            }

            if is_complete {
                self.verified_partial_profiles.insert((channel, descriptor));
                completed.push(ReassembledBunch {
                    channel,
                    descriptor,
                    first_sequence,
                    last_sequence,
                    fragment_count,
                    data_bit_len,
                    data,
                });
                consumed.extend(chain_keys);
            }
        }
        for key in &consumed {
            self.fragments.remove(key);
        }
        self.fragment_order.retain(|key| !consumed.contains(key));
        completed
    }

    fn drop_stale_fragments(&mut self) {
        let Some(latest) = self.latest_packet_order else {
            return;
        };
        let stale = self
            .fragment_order
            .iter()
            .copied()
            .filter(|key| {
                self.fragments
                    .get(key)
                    .is_none_or(|stored| latest - stored.packet_order > self.max_packet_span)
            })
            .collect::<HashSet<_>>();
        for key in &stale {
            self.fragments.remove(key);
        }
        self.fragment_order.retain(|key| !stale.contains(key));
    }
}

/// The scanned 13-bit prefix contains a 10-bit channel index and three header flags.
pub(crate) fn reliable_bunch_channel(prefix: u16) -> u16 {
    prefix & 0x03ff
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

fn is_supported_bunch_flags(partial_flags: u8) -> bool {
    matches!(partial_flags, 0x04 | 0x05 | 0x08 | 0x09 | 0x0c | 0x0d)
}

#[derive(Clone, Copy)]
struct BunchCandidate {
    bit_offset: usize,
    prefix: u16,
    sequence: u16,
    descriptor: u8,
    partial_flags: u8,
    data_bit_len: usize,
}

enum CandidateSlot {
    Unique(BunchCandidate),
    Ambiguous,
}

fn index_bunch_candidates(
    packet: &SequencedPacket,
    bunches_end: usize,
) -> HashMap<usize, CandidateSlot> {
    let mut candidates = HashMap::new();
    let Some(last_start) = bunches_end.checked_sub(BUNCH_HEADER_BITS) else {
        return candidates;
    };
    for bit_offset in 0..=last_start {
        let Some(bunch_flags) = read_bits_le(&packet.payload, bit_offset + 23, 12) else {
            continue;
        };
        let partial_flags = (bunch_flags & 0x0f) as u8;
        if !is_supported_bunch_flags(partial_flags) {
            continue;
        }
        let Some(data_bit_len) = read_bits_le(&packet.payload, bit_offset + 35, 13) else {
            continue;
        };
        let Some(data_end) = bit_offset
            .checked_add(BUNCH_HEADER_BITS)
            .and_then(|header_end| header_end.checked_add(data_bit_len as usize))
        else {
            continue;
        };
        if data_end > bunches_end {
            continue;
        }
        let Some(prefix) = read_bits_le(&packet.payload, bit_offset, 13) else {
            continue;
        };
        let Some(sequence) = read_bits_le(&packet.payload, bit_offset + 13, 10) else {
            continue;
        };
        let candidate = BunchCandidate {
            bit_offset,
            prefix: prefix as u16,
            sequence: sequence as u16,
            descriptor: (bunch_flags >> 4) as u8,
            partial_flags,
            data_bit_len: data_bit_len as usize,
        };
        candidates
            .entry(data_end)
            .and_modify(|slot| *slot = CandidateSlot::Ambiguous)
            .or_insert(CandidateSlot::Unique(candidate));
    }
    candidates
}

fn materialize_bunch(packet: &SequencedPacket, candidate: BunchCandidate) -> Option<LocatedBunch> {
    Some(LocatedBunch {
        bit_offset: candidate.bit_offset,
        bunch: SingleBunch {
            prefix: candidate.prefix,
            sequence: candidate.sequence,
            descriptor: candidate.descriptor,
            partial_flags: candidate.partial_flags,
            data_bit_len: candidate.data_bit_len,
            data: extract_bits(
                &packet.payload,
                candidate.bit_offset + BUNCH_HEADER_BITS,
                candidate.data_bit_len,
            )?,
        },
    })
}

/// Recovers continuation Bunches whose channel, reliable sequence and descriptor are all
/// established by a previously observed fragment. This is deliberately narrower than the
/// tail-anchored packet parser: payload bytes cannot create a new stream or change its identity.
pub fn parse_expected_bunch_continuations(
    packet: &SequencedPacket,
    expected: &[(u16, u16, u8)],
) -> Vec<SingleBunch> {
    if packet.mode != 0 || expected.is_empty() || packet.payload_bit_len < BUNCH_HEADER_BITS + 1 {
        return Vec::new();
    }
    let bunches_end = packet.payload_bit_len - 1;
    if read_bits_le(&packet.payload, bunches_end, 1) != Some(1) {
        return Vec::new();
    }
    let Some(last_start) = bunches_end.checked_sub(BUNCH_HEADER_BITS) else {
        return Vec::new();
    };
    let expected = expected.iter().copied().collect::<HashSet<_>>();
    let mut matches = Vec::new();
    for bit_offset in 0..=last_start {
        let Some(bunch_flags) = read_bits_le(&packet.payload, bit_offset + 23, 12) else {
            continue;
        };
        let partial_flags = (bunch_flags & 0x0f) as u8;
        if !matches!(partial_flags, 0x08 | 0x0c) {
            continue;
        }
        let Some(prefix) = read_bits_le(&packet.payload, bit_offset, 13) else {
            continue;
        };
        let Some(sequence) = read_bits_le(&packet.payload, bit_offset + 13, 10) else {
            continue;
        };
        let descriptor = (bunch_flags >> 4) as u8;
        let identity = (
            reliable_bunch_channel(prefix as u16),
            sequence as u16,
            descriptor,
        );
        if !expected.contains(&identity) {
            continue;
        }
        let Some(data_bit_len) = read_bits_le(&packet.payload, bit_offset + 35, 13) else {
            continue;
        };
        let Some(data_end) = bit_offset
            .checked_add(BUNCH_HEADER_BITS)
            .and_then(|header_end| header_end.checked_add(data_bit_len as usize))
        else {
            continue;
        };
        if data_bit_len == 0 || data_end > bunches_end {
            continue;
        }
        let candidate = BunchCandidate {
            bit_offset,
            prefix: prefix as u16,
            sequence: sequence as u16,
            descriptor,
            partial_flags,
            data_bit_len: data_bit_len as usize,
        };
        if let Some(located) = materialize_bunch(packet, candidate) {
            matches.push((bit_offset, located.bunch));
        }
    }
    matches.sort_by_key(|(bit_offset, _)| *bit_offset);
    let mut seen = HashSet::new();
    matches
        .into_iter()
        .filter_map(|(_, bunch)| {
            let identity = (
                reliable_bunch_channel(bunch.prefix),
                bunch.sequence,
                bunch.descriptor,
            );
            seen.insert(identity).then_some(bunch)
        })
        .collect()
}

/// Recovers initial partial Bunches only for channel/descriptor pairs that previously completed a
/// full reliable fragment chain. A recovered start remains inert until the exact next reliable
/// sequence arrives, so payload lookalikes cannot emit standalone application data.
pub fn parse_verified_bunch_starts(
    packet: &SequencedPacket,
    verified_profiles: &[(u16, u8)],
) -> Vec<SingleBunch> {
    if packet.mode != 0
        || verified_profiles.is_empty()
        || packet.payload_bit_len < BUNCH_HEADER_BITS + 1
    {
        return Vec::new();
    }
    let bunches_end = packet.payload_bit_len - 1;
    if read_bits_le(&packet.payload, bunches_end, 1) != Some(1) {
        return Vec::new();
    }
    let Some(last_start) = bunches_end.checked_sub(BUNCH_HEADER_BITS) else {
        return Vec::new();
    };
    let verified_profiles = verified_profiles.iter().copied().collect::<HashSet<_>>();
    let mut matches = Vec::new();
    for bit_offset in 0..=last_start {
        let Some(bunch_flags) = read_bits_le(&packet.payload, bit_offset + 23, 12) else {
            continue;
        };
        let partial_flags = (bunch_flags & 0x0f) as u8;
        if partial_flags != 0x09 {
            continue;
        }
        let Some(prefix) = read_bits_le(&packet.payload, bit_offset, 13) else {
            continue;
        };
        let descriptor = (bunch_flags >> 4) as u8;
        if !verified_profiles.contains(&(reliable_bunch_channel(prefix as u16), descriptor)) {
            continue;
        }
        let Some(sequence) = read_bits_le(&packet.payload, bit_offset + 13, 10) else {
            continue;
        };
        let Some(data_bit_len) = read_bits_le(&packet.payload, bit_offset + 35, 13) else {
            continue;
        };
        let Some(data_end) = bit_offset
            .checked_add(BUNCH_HEADER_BITS)
            .and_then(|header_end| header_end.checked_add(data_bit_len as usize))
        else {
            continue;
        };
        if data_bit_len == 0 || data_end > bunches_end {
            continue;
        }
        let candidate = BunchCandidate {
            bit_offset,
            prefix: prefix as u16,
            sequence: sequence as u16,
            descriptor,
            partial_flags,
            data_bit_len: data_bit_len as usize,
        };
        if let Some(located) = materialize_bunch(packet, candidate) {
            matches.push((bit_offset, located.bunch));
        }
    }
    matches.sort_by_key(|(bit_offset, _)| *bit_offset);
    let mut seen = HashSet::new();
    matches
        .into_iter()
        .filter_map(|(_, bunch)| {
            let identity = (
                reliable_bunch_channel(bunch.prefix),
                bunch.sequence,
                bunch.descriptor,
            );
            seen.insert(identity).then_some(bunch)
        })
        .collect()
}

/// Parses a packet-info prefix followed by one or more contiguous Bunches.
///
/// The trailing Bunch terminator is the anchor. Predecessors must end exactly where the next
/// Bunch begins. Ambiguous headers fail closed instead of choosing a convenient bit offset from
/// untrusted payload bytes.
pub fn parse_bunch_packet(packet: &SequencedPacket) -> Result<BunchPacket, BunchParseError> {
    if packet.mode != 0 {
        return Err(BunchParseError::UnsupportedMode);
    }
    if packet.payload_bit_len < BUNCH_HEADER_BITS + 1 {
        return Err(BunchParseError::PayloadTooShort);
    }
    let bunches_end = packet.payload_bit_len - 1;
    if read_bits_le(&packet.payload, bunches_end, 1) != Some(1) {
        return Err(BunchParseError::MissingTerminator);
    }

    let candidates = index_bunch_candidates(packet, bunches_end);
    let mut cursor = bunches_end;
    let mut reversed = Vec::new();
    loop {
        match candidates.get(&cursor) {
            Some(CandidateSlot::Unique(candidate)) => {
                if reversed.len() >= MAX_BUNCHES_PER_PACKET {
                    return Err(BunchParseError::TooManyBunches);
                }
                reversed.push(
                    materialize_bunch(packet, *candidate)
                        .ok_or(BunchParseError::PayloadTooShort)?,
                );
                cursor = candidate.bit_offset;
            }
            Some(CandidateSlot::Ambiguous) if reversed.is_empty() => {
                return Err(BunchParseError::AmbiguousTail);
            }
            Some(CandidateSlot::Ambiguous) => {
                return Err(BunchParseError::AmbiguousPredecessor);
            }
            None if reversed.is_empty() => return Err(BunchParseError::NoTailBunch),
            None => break,
        }
    }
    reversed.reverse();
    Ok(BunchPacket {
        packet_info_bit_len: cursor,
        bunches: reversed,
    })
}

fn unwrap_packet_id(packet_id: u16, reference: Option<i64>) -> i64 {
    const PACKET_ID_BITS: u32 = 14;
    const PACKET_ID_MODULUS: i64 = 1 << PACKET_ID_BITS;
    const PACKET_ID_HALF_RANGE: i64 = PACKET_ID_MODULUS / 2;
    const PACKET_ID_MASK: u16 = (1 << PACKET_ID_BITS) - 1;

    let raw = i64::from(packet_id & PACKET_ID_MASK);
    let Some(reference) = reference else {
        return raw;
    };
    let base = reference - reference.rem_euclid(PACKET_ID_MODULUS);
    let mut unwrapped = base + raw;
    if unwrapped - reference > PACKET_ID_HALF_RANGE {
        unwrapped -= PACKET_ID_MODULUS;
    } else if reference - unwrapped > PACKET_ID_HALF_RANGE {
        unwrapped += PACKET_ID_MODULUS;
    }
    unwrapped
}

fn append_bounded_bits(
    destination: &mut Vec<u8>,
    destination_bit_len: &mut usize,
    source: &[u8],
    source_bit_len: usize,
    max_bits: usize,
) -> Option<()> {
    if source_bit_len > source.len().checked_mul(8)? {
        return None;
    }
    let new_bit_len = destination_bit_len.checked_add(source_bit_len)?;
    if new_bit_len > max_bits {
        return None;
    }
    destination.resize(new_bit_len.div_ceil(8), 0);
    for index in 0..source_bit_len {
        let bit = (source[index / 8] >> (index % 8)) & 1;
        let target = *destination_bit_len + index;
        destination[target / 8] |= bit << (target % 8);
    }
    *destination_bit_len = new_bit_len;
    Some(())
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

    let mut channels = known_channels
        .iter()
        .copied()
        .map(reliable_bunch_channel)
        .collect::<HashSet<_>>();
    let mut exact_tail = Vec::new();
    for bit_offset in 0..=last_start {
        if let Some(bunch) = parse_inventory_bunch_at(packet, bit_offset, true) {
            channels.insert(reliable_bunch_channel(bunch.prefix));
            exact_tail.push((bit_offset, bunch));
        }
    }
    if channels.is_empty() {
        return Vec::new();
    }

    let exact_keys = exact_tail
        .iter()
        .map(|(_, bunch)| (reliable_bunch_channel(bunch.prefix), bunch.sequence))
        .collect::<HashSet<_>>();
    let mut candidates = exact_tail;
    for bit_offset in 0..=last_start {
        let Some(bunch) = parse_inventory_bunch_at(packet, bit_offset, false) else {
            continue;
        };
        let channel = reliable_bunch_channel(bunch.prefix);
        let key = (channel, bunch.sequence);
        if channels.contains(&channel) && !exact_keys.contains(&key) {
            candidates.push((bit_offset, bunch));
        }
    }

    candidates.sort_by_key(|(bit_offset, _)| *bit_offset);
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter_map(|(_, bunch)| {
            seen.insert((reliable_bunch_channel(bunch.prefix), bunch.sequence))
                .then_some(bunch)
        })
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
        header: (u16, u16, u8, u8),
        data: &[u8],
        data_bit_len: usize,
    ) -> usize {
        let (prefix, sequence, descriptor, partial_flags) = header;
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
            (4122, 550, INVENTORY_BUNCH_DESCRIPTOR, 0x09),
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
            (5146, 100, INVENTORY_BUNCH_DESCRIPTOR, 0x09),
            &first_data,
            16,
        );
        let data_end = write_bunch(
            &mut payload,
            second_offset,
            (4122, 101, INVENTORY_BUNCH_DESCRIPTOR, 0x0c),
            &second_data,
            11,
        );
        write_bits(&mut payload, data_end, 1, 1);
        let packet = sequenced_packet(payload, payload_bit_len);

        let bunches = parse_inventory_bunches(&packet, &[]);

        assert_eq!(bunches.len(), 2);
        assert_eq!(bunches[0].prefix, 5146);
        assert_eq!(bunches[0].sequence, 100);
        assert_eq!(bunches[0].data, first_data);
        assert_eq!(bunches[1].prefix, 4122);
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
            (4122, 87, INVENTORY_BUNCH_DESCRIPTOR, 0x08),
            &[0x5a],
            8,
        );
        let packet = sequenced_packet(payload, payload_bit_len);

        assert!(parse_inventory_bunches(&packet, &[]).is_empty());
        assert_eq!(parse_inventory_bunches(&packet, &[5146]).len(), 1);
    }

    #[test]
    fn verified_profile_recovers_embedded_partial_start_and_expected_continuation() {
        let start_offset = 9;
        let start_data_end = start_offset + BUNCH_HEADER_BITS + 8;
        let start_payload_bits = start_data_end + 13 + 1;
        let mut start_payload = vec![0_u8; start_payload_bits.div_ceil(8)];
        write_bunch(
            &mut start_payload,
            start_offset,
            (4122, 200, INVENTORY_BUNCH_DESCRIPTOR, 0x09),
            &[0x5a],
            8,
        );
        write_bits(&mut start_payload, start_payload_bits - 1, 1, 1);
        let start_packet = sequenced_packet(start_payload, start_payload_bits);

        let starts =
            parse_verified_bunch_starts(&start_packet, &[(26, INVENTORY_BUNCH_DESCRIPTOR)]);
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].sequence, 200);
        assert_eq!(starts[0].data, [0x5a]);

        let continuation_offset = 7;
        let continuation_data_end = continuation_offset + BUNCH_HEADER_BITS + 8;
        let continuation_payload_bits = continuation_data_end + 17 + 1;
        let mut continuation_payload = vec![0_u8; continuation_payload_bits.div_ceil(8)];
        write_bunch(
            &mut continuation_payload,
            continuation_offset,
            (4122, 201, INVENTORY_BUNCH_DESCRIPTOR, 0x0c),
            &[0xa5],
            8,
        );
        write_bits(
            &mut continuation_payload,
            continuation_payload_bits - 1,
            1,
            1,
        );
        let continuation_packet = sequenced_packet(continuation_payload, continuation_payload_bits);

        let continuations = parse_expected_bunch_continuations(
            &continuation_packet,
            &[(26, 201, INVENTORY_BUNCH_DESCRIPTOR)],
        );
        assert_eq!(continuations.len(), 1);
        assert_eq!(continuations[0].sequence, 201);
        assert_eq!(continuations[0].data, [0xa5]);
    }

    #[test]
    fn rejects_invalid_inventory_flags_and_descriptor() {
        for (descriptor, partial_flags) in [(INVENTORY_BUNCH_DESCRIPTOR, 0x0a), (0xcb, 0x09)] {
            let payload_bit_len = BUNCH_HEADER_BITS + 8 + 1;
            let mut payload = vec![0_u8; payload_bit_len.div_ceil(8)];
            let data_end = write_bunch(
                &mut payload,
                0,
                (4122, 87, descriptor, partial_flags),
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

    #[test]
    fn parses_packet_info_and_multiple_non_byte_aligned_bunches() {
        let packet_info_bits = 19;
        let first_data = [0x5a, 0x01];
        let second_data = [0xa5, 0x05];
        let first_offset = packet_info_bits;
        let second_offset = first_offset + BUNCH_HEADER_BITS + 9;
        let payload_bit_len = second_offset + BUNCH_HEADER_BITS + 11 + 1;
        let mut payload = vec![0_u8; payload_bit_len.div_ceil(8)];
        write_bits(&mut payload, 0, packet_info_bits, 0x55aa);
        write_bunch(
            &mut payload,
            first_offset,
            (1027, 87, 0x38, 0x05),
            &first_data,
            9,
        );
        let data_end = write_bunch(
            &mut payload,
            second_offset,
            (4122, 88, INVENTORY_BUNCH_DESCRIPTOR, 0x09),
            &second_data,
            11,
        );
        write_bits(&mut payload, data_end, 1, 1);

        let parsed = parse_bunch_packet(&sequenced_packet(payload, payload_bit_len))
            .expect("a packet-info prefix and two contiguous Bunches should parse");

        assert_eq!(parsed.packet_info_bit_len, packet_info_bits);
        assert_eq!(parsed.bunches.len(), 2);
        assert_eq!(parsed.bunches[0].bit_offset, first_offset);
        assert_eq!(parsed.bunches[0].bunch.sequence, 87);
        assert_eq!(parsed.bunches[0].bunch.descriptor, 0x38);
        assert_eq!(parsed.bunches[0].bunch.data_bit_len, 9);
        assert_eq!(parsed.bunches[1].bit_offset, second_offset);
        assert_eq!(parsed.bunches[1].bunch.sequence, 88);
        assert_eq!(parsed.bunches[1].bunch.data_bit_len, 11);
    }

    #[test]
    fn bunch_packet_rejects_missing_terminator_and_unsupported_mode() {
        let mut packet = single_bunch_packet(0, 16, &[0x5a, 0xa5]);
        let terminator = packet.payload_bit_len - 1;
        packet.payload[terminator / 8] &= !(1 << (terminator % 8));
        assert_eq!(
            parse_bunch_packet(&packet),
            Err(BunchParseError::MissingTerminator)
        );

        let packet = single_bunch_packet(1, 16, &[0x5a, 0xa5]);
        assert_eq!(
            parse_bunch_packet(&packet),
            Err(BunchParseError::UnsupportedMode)
        );
    }

    fn fragment(sequence: u16, descriptor: u8, partial_flags: u8, data: u8) -> SingleBunch {
        SingleBunch {
            prefix: 4122,
            sequence,
            descriptor,
            partial_flags,
            data_bit_len: 8,
            data: vec![data],
        }
    }

    #[test]
    fn reassembles_out_of_order_partial_bunches_and_deduplicates_retransmission() {
        let mut reassembler = BunchReassembler::new(16, 128, 8);

        assert!(
            reassembler
                .observe_packet(11, [fragment(101, 0xcc, 0x08, 0x22)])
                .is_empty()
        );
        assert!(
            reassembler
                .observe_packet(12, [fragment(102, 0xcc, 0x0c, 0x33)])
                .is_empty()
        );
        let completed = reassembler.observe_packet(
            13,
            [
                fragment(100, 0xcc, 0x09, 0x11),
                fragment(101, 0xcc, 0x08, 0x22),
            ],
        );

        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].channel, 26);
        assert_eq!(completed[0].first_sequence, 100);
        assert_eq!(completed[0].last_sequence, 102);
        assert_eq!(completed[0].fragment_count, 3);
        assert_eq!(completed[0].data_bit_len, 24);
        assert_eq!(completed[0].data, [0x11, 0x22, 0x33]);
        assert_eq!(reassembler.verified_partial_profiles(), [(26, 0xcc)]);
        assert!(
            reassembler
                .observe_packet(14, [fragment(102, 0xcc, 0x0c, 0x33)])
                .is_empty()
        );
    }

    #[test]
    fn completes_multiple_fragment_streams_from_one_packet() {
        let mut reassembler = BunchReassembler::new(16, 128, 8);
        assert!(
            reassembler
                .observe_packet(
                    1,
                    [
                        fragment(10, 0xcb, 0x09, 0x11),
                        fragment(20, 0xcc, 0x09, 0x22),
                    ],
                )
                .is_empty()
        );

        let completed = reassembler.observe_packet(
            2,
            [
                fragment(11, 0xcb, 0x0c, 0x33),
                fragment(21, 0xcc, 0x0c, 0x44),
            ],
        );

        assert_eq!(completed.len(), 2);
        assert_eq!(completed[0].data, [0x11, 0x33]);
        assert_eq!(completed[1].data, [0x22, 0x44]);
    }

    #[test]
    fn replacing_fragment_at_capacity_keeps_other_streams() {
        let mut reassembler = BunchReassembler::new(2, 128, 8);
        assert!(
            reassembler
                .observe_packet(
                    1,
                    [
                        fragment(10, 0xcb, 0x09, 0x11),
                        fragment(20, 0xcc, 0x09, 0x22),
                    ],
                )
                .is_empty()
        );

        assert!(
            reassembler
                .observe_packet(2, [fragment(10, 0xcb, 0x09, 0x33)])
                .is_empty()
        );

        assert_eq!(
            reassembler.expected_continuations(),
            [(26, 11, 0xcb), (26, 21, 0xcc)]
        );
    }

    #[test]
    fn reassembler_fails_closed_on_descriptor_change_and_stream_limit() {
        let mut reassembler = BunchReassembler::new(16, 16, 8);
        assert!(
            reassembler
                .observe_packet(1, [fragment(10, 0xcc, 0x09, 0x11)])
                .is_empty()
        );
        assert!(
            reassembler
                .observe_packet(2, [fragment(11, 0xcb, 0x0c, 0x22)])
                .is_empty()
        );

        let mut reassembler = BunchReassembler::new(16, 8, 8);
        assert!(
            reassembler
                .observe_packet(1, [fragment(10, 0xcc, 0x09, 0x11)])
                .is_empty()
        );
        assert!(
            reassembler
                .observe_packet(2, [fragment(11, 0xcc, 0x0c, 0x22)])
                .is_empty()
        );
    }
}
