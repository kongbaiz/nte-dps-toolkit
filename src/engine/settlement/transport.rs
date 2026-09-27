//! Explicit-profile UE packet/Bunch/content-block framing for the new parser.
//! A profile is an input, never discovered by choosing whichever parse succeeds.
//! Caller owns connection/generation identity; one decoder cannot mix connections.
use super::wire::Bits;
use super::{Error, Name, Request, Settlement, decode_request, decode_settlement};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Profile {
    pub component_prefix: u8,
    pub channel: u32,
    pub field_upper_exclusive: u32,
    pub request_index: u32,
    pub settlement_index: u32,
}

#[derive(Debug)]
pub enum Rpc {
    Inventory {
        data: Vec<u8>,
        bits: usize,
    },
    Request(Request),
    Settlement(Settlement),
    /// A bounded RPC body whose extra-damage array is not yet supported.
    /// Preserve surrounding RPCs and sequence state; do not invent a hit.
    UnsupportedSettlementExtras,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Bunch {
    data: Vec<u8>,
    bits: usize,
    sequence: u32,
    reliable: bool,
    exports: bool,
    must_map: bool,
    open: bool,
    partial: bool,
    initial: bool,
    final_part: bool,
    aux: bool,
}
struct Pending {
    data: Vec<u8>,
    bits: usize,
    sequence: u32,
    reliable: bool,
    open: bool,
}

pub struct Decoder {
    profile: Profile,
    pending: [Option<Pending>; 2],
    // Reliable Bunches can be retransmitted in a different packet. Retain a
    // bounded exact-byte window (well below the 1024 sequence modulus), not
    // packet IDs or damage values, to avoid assembling a duplicate tail twice.
    reliable_seen: [VecDeque<Bunch>; 2],
    pub(super) saw_channel: bool,
    inventory_mode: bool,
    pub(crate) controller_bound: bool,
    controller_actor: u64,
    exports: std::collections::HashMap<u64, (u64, String, Option<u32>)>,
}

impl Decoder {
    pub fn new(profile: Profile) -> Result<Self, Error> {
        if profile.component_prefix >= 64
            || profile.field_upper_exclusive < 2
            || profile.field_upper_exclusive > 65536
            || profile.request_index >= profile.field_upper_exclusive
            || profile.settlement_index >= profile.field_upper_exclusive
            || profile.request_index == profile.settlement_index
        {
            return Err(Error::InvalidValue);
        }
        Ok(Self {
            profile,
            pending: [None, None],
            reliable_seen: Default::default(),
            saw_channel: false,
            inventory_mode: false,
            controller_bound: false,
            controller_actor: 0,
            exports: Default::default(),
        })
    }
    pub(crate) fn inventory(prefix: u8) -> Result<Self, Error> {
        let mut decoder = Self::new(Profile {
            component_prefix: prefix,
            channel: 3,
            field_upper_exclusive: 219,
            request_index: 100,
            settlement_index: 142,
        })?;
        decoder.inventory_mode = true;
        Ok(decoder)
    }
    pub fn has_incomplete_fragments(&self) -> bool {
        self.pending.iter().any(Option::is_some)
    }
    pub fn clear(&mut self) {
        self.pending = [None, None];
        self.reliable_seen.iter_mut().for_each(VecDeque::clear);
        self.exports.clear();
        self.controller_bound = false;
        self.controller_actor = 0;
    }

    pub(super) fn discard_incomplete(&mut self) {
        self.pending = [None, None];
    }

    pub fn datagram(&mut self, data: &[u8], inbound: bool) -> Result<Vec<Rpc>, Error> {
        self.saw_channel = false;
        if data.len() > 65535 {
            return Err(Error::BudgetExceeded);
        }
        let outer_len = termination(data)?;
        let mut outer = Bits::new(data, outer_len)?;
        let prefix = outer.take(6)? as u8;
        // The non-data branch precedes sequenced traffic on login. It carries
        // no Bunch/RPC stream and must not be interpreted as a sequenced header.
        if prefix == (self.profile.component_prefix | 32) && self.profile.component_prefix & 32 == 0
        {
            return Ok(Vec::new());
        }
        if prefix != self.profile.component_prefix {
            return Err(Error::Unsupported);
        }
        let intermediate = blob(&mut outer, outer_len - 6)?;
        let inner_len = termination(&intermediate)?;
        let mut r = Bits::new(&intermediate, inner_len)?;
        let word = r.take(32)? as u32;
        let history = (word & 15) + 1;
        if history > 8 {
            return Err(Error::Unsupported);
        }
        for _ in 0..history {
            r.take(32)?;
        }
        if r.take(1)? != 0 {
            bounded(&mut r, 1024)?;
            if r.take(1)? != 0 && inbound {
                r.take(8)?;
            }
        }
        // Parse the whole packet before changing fragment state.
        let mut bunches = Vec::new();
        while r.pos < r.len {
            let control = r.take(1)? != 0;
            let (open, close) = if control {
                (r.take(1)? != 0, r.take(1)? != 0)
            } else {
                (false, false)
            };
            if close {
                bounded(&mut r, 15)?;
            }
            r.take(1)?;
            let reliable = r.take(1)? != 0;
            let channel = r.packed()?;
            let exports = r.take(1)? != 0;
            let must_map = r.take(1)? != 0;
            let partial = r.take(1)? != 0;
            let sequence = if reliable {
                bounded(&mut r, 1024)?
            } else {
                word >> 18
            };
            let (initial, aux, final_part) = if partial {
                (r.take(1)? != 0, r.take(1)? != 0, r.take(1)? != 0)
            } else {
                (false, false, false)
            };
            if (open || reliable)
                && let Name::Hardcoded(i) = r.name()?
                && i >= 0x2bf
            {
                return Err(Error::InvalidValue);
            }
            let bits = bounded(&mut r, 8192)? as usize;
            let payload = blob(&mut r, bits)?;
            if channel == self.profile.channel {
                bunches.push(Bunch {
                    data: payload,
                    bits,
                    sequence,
                    reliable,
                    exports,
                    must_map,
                    open,
                    partial,
                    initial,
                    final_part,
                    aux,
                });
            }
        }
        let mut result = Vec::new();
        self.saw_channel = !bunches.is_empty();
        for b in bunches {
            if self.reliable_duplicate(&b, inbound)? {
                continue;
            }
            if self.inventory_mode && b.exports {
                let mut r = Bits::new(&b.data, b.bits)?;
                if r.take(1)? != 0 {
                    return Err(Error::Unsupported);
                }
                let count = r.take(32)?;
                if count > 2048 {
                    return Err(Error::BudgetExceeded);
                }
                for _ in 0..count {
                    self.export_guid(&mut r, 0)?;
                }
                if r.pos != r.len {
                    return Err(Error::InvalidValue);
                }
            }
            if !b.partial {
                if !b.exports && b.bits > 0 {
                    result.extend(self.message(&b.data, b.bits, b.must_map, b.open, inbound)?);
                }
                continue;
            }
            let slot = usize::from(inbound);
            if b.initial {
                if self.pending[slot].is_some() {
                    self.pending[slot] = None;
                    return Err(Error::InvalidValue);
                }
                self.pending[slot] = Some(Pending {
                    data: vec![],
                    bits: 0,
                    sequence: b.sequence,
                    reliable: b.reliable,
                    open: b.open,
                });
            }
            let Some(mut pending) = self.pending[slot].take() else {
                return Err(Error::Truncated);
            };
            if !b.initial {
                let modulus = if b.reliable { 1024 } else { 16384 };
                let delta = (b.sequence + modulus - pending.sequence) % modulus;
                if pending.reliable != b.reliable || (delta != 1 && (b.reliable || delta != 0)) {
                    return Err(Error::InvalidValue);
                }
            }
            if !b.exports {
                if !b.final_part && !b.aux && b.bits % 8 != 0 {
                    return Err(Error::InvalidValue);
                }
                append(&mut pending.data, pending.bits, &b.data, b.bits)?;
                pending.bits += b.bits;
            }
            pending.sequence = b.sequence;
            if b.final_part {
                if pending.bits > 0 {
                    result.extend(self.message(
                        &pending.data,
                        pending.bits,
                        b.must_map,
                        pending.open,
                        inbound,
                    )?);
                }
            } else {
                self.pending[slot] = Some(pending);
            }
        }
        Ok(result)
    }

    fn reliable_duplicate(&mut self, bunch: &Bunch, inbound: bool) -> Result<bool, Error> {
        if !bunch.reliable {
            return Ok(false);
        }
        let seen = &mut self.reliable_seen[usize::from(inbound)];
        if let Some(previous) = seen
            .iter()
            .find(|previous| previous.sequence == bunch.sequence)
        {
            return if previous == bunch {
                Ok(true)
            } else {
                Err(Error::InvalidValue)
            };
        }
        if seen.len() == 256 {
            seen.pop_front();
        }
        seen.push_back(bunch.clone());
        Ok(false)
    }

    fn message(
        &mut self,
        data: &[u8],
        bits: usize,
        must_map: bool,
        open: bool,
        inbound: bool,
    ) -> Result<Vec<Rpc>, Error> {
        let mut r = Bits::new(data, bits)?;
        if must_map {
            if !inbound {
                return Err(Error::Unsupported);
            }
            let n = r.take(16)?;
            if n > 1024 {
                return Err(Error::BudgetExceeded);
            }
            for _ in 0..n {
                packed64(&mut r)?;
            }
        }
        if open {
            let guid = object(&mut r, 0)?;
            if self.inventory_mode && self.controller_actor != 0 && self.controller_actor != guid {
                return Err(Error::InvalidValue);
            }
            if self.inventory_mode {
                self.controller_actor = guid;
            }
            // Consume the verified new-actor framing before content blocks.
            // These fields are framing only, never damage/HP evidence.
            if guid != 0 && guid & 1 == 0 {
                let archetype = object(&mut r, 0)?;
                if self.inventory_mode {
                    self.controller_bound = self.export_path(archetype, 0).as_deref()
                        == Some(
                            "/Game/Blueprints/Share/Character/Player/BP_PlayerControllerBase.Default__BP_PlayerControllerBase_C",
                        )
                        && self
                            .exports
                            .get(&archetype)
                            .is_some_and(|v| v.2 == Some(3604383830));
                }
                object(&mut r, 0)?; // level
                spawn_vector(&mut r)?;
                if r.take(1)? != 0 {
                    for _ in 0..3 {
                        if r.take(1)? != 0 {
                            r.take(16)?;
                        }
                    }
                }
                spawn_vector(&mut r)?; // scale
                spawn_vector(&mut r)?; // velocity
            }
        }
        let mut result = Vec::new();
        while r.pos < r.len {
            let layout = r.take(1)? != 0;
            let actor = r.take(1)? != 0;
            let mut deleted = false;
            if !actor {
                object(&mut r, 0)?;
                if inbound && r.take(1)? == 0 {
                    deleted = r.take(1)? != 0;
                    if deleted {
                        r.take(8)?;
                    } else {
                        deleted = object(&mut r, 0)? == 0;
                        if !deleted && r.take(1)? == 0 {
                            object(&mut r, 0)?;
                        }
                    }
                }
            }
            let n = if deleted { 0 } else { r.packed()? as usize };
            let payload = blob(&mut r, n)?;
            if !actor || layout || deleted {
                continue;
            }
            let mut f = Bits::new(&payload, n)?;
            while f.pos < f.len {
                let index = bounded(&mut f, self.profile.field_upper_exclusive)?;
                let count = f.packed()? as usize;
                let raw = blob(&mut f, count)?;
                if self.inventory_mode {
                    if self.controller_bound && inbound && index == 131 {
                        result.push(Rpc::Inventory {
                            data: raw,
                            bits: count,
                        });
                    }
                    continue;
                }
                if index == self.profile.request_index {
                    if inbound {
                        return Err(Error::InvalidValue);
                    }
                    result.push(Rpc::Request(decode_request(
                        &raw,
                        count,
                        self.profile.channel,
                    )?));
                } else if index == self.profile.settlement_index {
                    if !inbound {
                        return Err(Error::InvalidValue);
                    }
                    result.push(match decode_settlement(&raw, count, self.profile.channel) {
                        Ok(settlement) => Rpc::Settlement(settlement),
                        Err(Error::UnsupportedSettlementExtras) => Rpc::UnsupportedSettlementExtras,
                        Err(error) => return Err(error),
                    });
                }
            }
        }
        Ok(result)
    }
    fn export_guid(&mut self, r: &mut Bits<'_>, depth: usize) -> Result<u64, Error> {
        if depth > 16 || self.exports.len() > 4096 {
            return Err(Error::BudgetExceeded);
        }
        let guid = packed64(r)?;
        if guid == 0 {
            return Ok(guid);
        }
        let flags = r.take(8)?;
        if flags & 1 != 0 {
            let outer = self.export_guid(r, depth + 1)?;
            let name = r.string()?;
            if name.len() > 512 {
                return Err(Error::BudgetExceeded);
            }
            let checksum = if flags & 4 != 0 {
                Some(r.take(32)? as u32)
            } else {
                None
            };
            let value = (outer, name, checksum);
            if self.exports.get(&guid).is_some_and(|old| old != &value) {
                return Err(Error::InvalidValue);
            }
            self.exports.insert(guid, value);
        }
        Ok(guid)
    }
    fn export_path(&self, guid: u64, depth: usize) -> Option<String> {
        if depth > 16 {
            return None;
        }
        let (outer, name, _) = self.exports.get(&guid)?;
        if *outer == 0 {
            Some(name.clone())
        } else {
            Some(format!("{}.{}", self.export_path(*outer, depth + 1)?, name))
        }
    }
}

fn spawn_vector(r: &mut Bits<'_>) -> Result<(), Error> {
    if r.take(1)? == 0 {
        return Ok(());
    }
    if r.take(1)? == 0 {
        for _ in 0..3 {
            r.take(64)?;
        }
        return Ok(());
    }
    let header = bounded(r, 128)?;
    let width = header & 63;
    let width = if width != 0 {
        width
    } else if header & 64 != 0 {
        64
    } else {
        32
    };
    for _ in 0..3 {
        r.take(width as usize)?;
    }
    Ok(())
}

fn termination(data: &[u8]) -> Result<usize, Error> {
    let byte = *data.last().ok_or(Error::Truncated)?;
    if byte == 0 {
        return Err(Error::InvalidValue);
    }
    Ok((data.len() - 1) * 8 + 7 - byte.leading_zeros() as usize)
}
fn bounded(r: &mut Bits<'_>, max: u32) -> Result<u32, Error> {
    let (mut value, mut mask) = (0u32, 1u32);
    while value + mask < max {
        value |= (r.take(1)? as u32) * mask;
        mask <<= 1;
    }
    Ok(value)
}
fn blob(r: &mut Bits<'_>, n: usize) -> Result<Vec<u8>, Error> {
    if n > r.len - r.pos {
        return Err(Error::Truncated);
    }
    let mut data = vec![0u8; n.div_ceil(8)];
    for (i, b) in data.iter_mut().enumerate() {
        *b = r.take((n - i * 8).min(8))? as u8;
    }
    Ok(data)
}
fn append(dst: &mut Vec<u8>, offset: usize, src: &[u8], bits: usize) -> Result<(), Error> {
    if offset + bits > 8 * 1024 * 1024 {
        return Err(Error::BudgetExceeded);
    }
    dst.resize((offset + bits).div_ceil(8), 0);
    for i in 0..bits {
        dst[(offset + i) / 8] |= ((src[i / 8] >> (i % 8)) & 1) << ((offset + i) % 8);
    }
    Ok(())
}
fn packed64(r: &mut Bits<'_>) -> Result<u64, Error> {
    let mut v = 0;
    for i in 0..10 {
        let b = r.take(8)?;
        if i == 9 && b >> 1 > 1 {
            return Err(Error::InvalidValue);
        }
        v |= (b >> 1) << (i * 7);
        if b & 1 == 0 {
            return Ok(v);
        }
    }
    Err(Error::InvalidValue)
}
fn object(r: &mut Bits<'_>, depth: usize) -> Result<u64, Error> {
    if depth > 16 {
        return Err(Error::BudgetExceeded);
    }
    let guid = packed64(r)?;
    if guid == 1 {
        let flags = r.take(8)?;
        if flags & 1 != 0 {
            object(r, depth + 1)?;
            r.string()?;
            if flags & 4 != 0 {
                r.take(32)?;
            }
        }
    }
    Ok(guid)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    fn decoder() -> Decoder {
        Decoder::new(Profile {
            component_prefix: 28,
            channel: 3,
            field_upper_exclusive: 219,
            request_index: 100,
            settlement_index: 142,
        })
        .unwrap()
    }
    #[test]
    fn inventory_requires_captured_archetype_checksum_and_fixed_actor() {
        let mut d = Decoder::inventory(28).unwrap();
        d.exports.insert(
            11,
            (
                0,
                "/Game/Blueprints/Share/Character/Player/BP_PlayerControllerBase".into(),
                None,
            ),
        );
        d.exports.insert(
            9,
            (11, "Default__BP_PlayerControllerBase_C".into(), Some(0)),
        );
        d.message(&[4, 18, 6, 0], 28, false, true, true).unwrap();
        assert!(!d.controller_bound);
        d.exports.get_mut(&9).unwrap().2 = Some(3604383830);
        d.message(&[4, 18, 6, 0], 28, false, true, true).unwrap();
        assert!(d.controller_bound);
        assert!(d.message(&[8, 18, 6, 0], 28, false, true, true).is_err());
    }

    fn bunch(sequence: u32) -> Bunch {
        Bunch {
            data: vec![0],
            bits: 2,
            sequence,
            reliable: true,
            exports: false,
            must_map: false,
            open: false,
            partial: true,
            initial: false,
            final_part: true,
            aux: false,
        }
    }

    #[derive(Default)]
    struct Writer {
        data: Vec<u8>,
        bits: usize,
    }
    impl Writer {
        fn put(&mut self, value: u64, bits: usize) {
            self.data.resize((self.bits + bits).div_ceil(8), 0);
            for i in 0..bits {
                self.data[(self.bits + i) / 8] |=
                    (((value >> i) & 1) as u8) << ((self.bits + i) % 8);
            }
            self.bits += bits;
        }
        fn finish(mut self) -> Vec<u8> {
            self.put(1, 1);
            self.data
        }
    }

    pub(crate) fn fragment(
        sequence: u32,
        initial: bool,
        final_part: bool,
        payload: u8,
        bits: usize,
    ) -> Vec<u8> {
        let mut w = Writer::default();
        w.put(u64::from(sequence) << 18, 32);
        w.put(0, 32);
        w.put(0, 1);
        w.put(0, 1);
        w.put(0, 1);
        w.put(1, 1); // control, paused, reliable
        w.put(6, 8); // packed channel 3
        w.put(0, 1);
        w.put(0, 1);
        w.put(1, 1); // exports, must-map, partial
        w.put(u64::from(sequence), 10);
        w.put(u64::from(initial), 1);
        w.put(0, 1);
        w.put(u64::from(final_part), 1);
        w.put(1, 1);
        w.put(0, 8); // hardcoded channel name
        w.put(bits as u64, 13);
        w.put(u64::from(payload), bits);
        let inner = w.finish();
        let mut outer = Writer::default();
        outer.put(28, 6);
        for byte in inner {
            outer.put(u64::from(byte), 8);
        }
        outer.finish()
    }

    #[test]
    fn unsupported_bounded_rpc_preserves_later_rpc_in_same_content_block() {
        fn packed(w: &mut Writer, mut n: usize) {
            loop {
                let next = n >> 7;
                w.put((((n & 127) << 1) | usize::from(next != 0)) as u64, 8);
                n = next;
                if n == 0 {
                    break;
                }
            }
        }
        let mut fields = Writer::default();
        for extra in [1, 0] {
            let (raw, bits) = super::super::wire::tests::recovery_rpc(extra, 1, 800.0);
            let mut value = 0;
            let mut mask = 1;
            while value + mask < 219 {
                let bit = 142 & mask != 0;
                fields.put(u64::from(bit), 1);
                if bit {
                    value |= mask
                }
                mask <<= 1;
            }
            packed(&mut fields, bits);
            for i in 0..bits {
                fields.put(u64::from((raw[i / 8] >> (i % 8)) & 1), 1);
            }
        }
        let mut content = Writer::default();
        content.put(0, 1);
        content.put(1, 1);
        packed(&mut content, fields.bits);
        for i in 0..fields.bits {
            content.put(u64::from((fields.data[i / 8] >> (i % 8)) & 1), 1);
        }
        let mut d = decoder();
        let r = d
            .message(&content.data, content.bits, false, false, true)
            .unwrap();
        assert_eq!(r.len(), 2);
        assert!(matches!(r[0], Rpc::UnsupportedSettlementExtras));
        assert!(
            matches!(&r[1],Rpc::Settlement(s) if s.targets.is_empty() && s.recoveries.len()==1)
        );
    }

    #[test]
    fn reliable_retransmitted_tail_is_not_a_missing_first_fragment() {
        let mut d = decoder();
        let first = fragment(10, true, false, 3, 8);
        let last = fragment(11, false, true, 0, 2);
        d.datagram(&first, true).unwrap();
        assert!(d.has_incomplete_fragments());
        d.datagram(&first, true).unwrap(); // duplicate initial keeps the group
        d.datagram(&last, true).unwrap();
        assert!(!d.has_incomplete_fragments());
        assert!(d.datagram(&last, true).unwrap().is_empty());
        d.datagram(&fragment(12, true, false, 3, 8), true).unwrap();
        d.datagram(&fragment(13, false, true, 0, 2), true).unwrap();
        assert!(!d.has_incomplete_fragments());
    }

    #[test]
    fn genuinely_missing_or_discontinuous_fragment_still_fails() {
        assert!(matches!(
            decoder().datagram(&fragment(11, false, true, 0, 2), true),
            Err(Error::Truncated)
        ));
        let mut d = decoder();
        d.datagram(&fragment(10, true, false, 3, 8), true).unwrap();
        assert!(matches!(
            d.datagram(&fragment(12, false, true, 0, 2), true),
            Err(Error::InvalidValue)
        ));
    }

    #[test]
    fn duplicate_identity_is_directional_exact_and_bounded_across_sequence_wrap() {
        let mut d = decoder();
        let b = bunch(10);
        assert!(!d.reliable_duplicate(&b, true).unwrap());
        assert!(!d.reliable_duplicate(&b, false).unwrap());
        assert!(d.reliable_duplicate(&b, true).unwrap());
        let mut conflicting = b.clone();
        conflicting.data[0] = 1;
        assert!(matches!(
            d.reliable_duplicate(&conflicting, true),
            Err(Error::InvalidValue)
        ));
        d.clear();
        for i in 0..2048 {
            assert!(!d.reliable_duplicate(&bunch(i % 1024), true).unwrap());
        }
        assert_eq!(d.reliable_seen[1].len(), 256);
        d.clear();
        assert!(d.reliable_seen.iter().all(VecDeque::is_empty));
        assert!(!d.reliable_duplicate(&b, true).unwrap());
    }

    #[test]
    fn non_data_branch_produces_no_rpc_and_other_prefix_still_fails() {
        let mut d = decoder();
        assert!(d.datagram(&[60 | 64], true).unwrap().is_empty());
        assert!(matches!(
            d.datagram(&[20 | 64], true),
            Err(Error::Unsupported)
        ));
        assert!(matches!(d.datagram(&[], true), Err(Error::Truncated)));
    }

    #[test]
    fn dynamic_actor_spawn_header_is_consumed_but_truncation_fails() {
        let mut d = decoder();
        assert!(
            d.message(&[4, 18, 6, 0], 28, false, true, true)
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            d.message(&[4, 18, 6, 0], 27, false, true, true),
            Err(Error::Truncated)
        ));
    }

    #[test]
    fn spawn_vector_forms_consume_only_their_declared_bits() {
        for header in [0, 64, 22, 86] {
            let mut w = Writer::default();
            w.put(1, 1);
            w.put(1, 1);
            w.put(header, 7);
            let width = match header {
                0 => 32,
                64 => 64,
                _ => 22,
            };
            for _ in 0..3 {
                w.put(0, width);
            }
            let mut r = Bits::new(&w.data, w.bits).unwrap();
            spawn_vector(&mut r).unwrap();
            assert_eq!(r.pos, w.bits);
            let mut truncated = Bits::new(&w.data, w.bits - 1).unwrap();
            assert!(spawn_vector(&mut truncated).is_err());
        }
        let mut w = Writer::default();
        w.put(1, 1);
        w.put(0, 1);
        for _ in 0..3 {
            w.put(0, 64);
        }
        let mut r = Bits::new(&w.data, w.bits).unwrap();
        spawn_vector(&mut r).unwrap();
        assert_eq!(r.pos, w.bits);
    }
}
