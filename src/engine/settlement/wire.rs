//! Native v6/v7 RPC parameter grammar, including the qualified v7 damage triples.
//! Unknown branches/extra arrays fail visibly; no arbitrary byte-pattern search.
use super::*;

const MAX_ITEMS: usize = 1024;
const MAX_RPC_BITS: usize = 8 * 1024 * 1024;
const V6_REQUEST_FLAGS_BITS: usize = 5;
const V7_REQUEST_FLAGS_BITS: usize = 7;
const V7_DAMAGE_WRAPPERS_MARKER: u64 = 1;

pub(crate) struct Bits<'a> {
    pub(crate) data: &'a [u8],
    pub(crate) pos: usize,
    pub(crate) len: usize,
}
impl<'a> Bits<'a> {
    pub(crate) fn new(data: &'a [u8], len: usize) -> Result<Self, Error> {
        if len > MAX_RPC_BITS {
            return Err(Error::BudgetExceeded);
        }
        if len > data.len().saturating_mul(8) {
            return Err(Error::Truncated);
        }
        Ok(Self { data, pos: 0, len })
    }
    pub(crate) fn take(&mut self, n: usize) -> Result<u64, Error> {
        if n > 64 || n > self.len - self.pos {
            return Err(Error::Truncated);
        }
        let mut out = 0;
        for i in 0..n {
            out |= u64::from((self.data[(self.pos + i) / 8] >> ((self.pos + i) % 8)) & 1) << i;
        }
        self.pos += n;
        Ok(out)
    }
    pub(crate) fn bytes(&mut self, n: usize) -> Result<Vec<u8>, Error> {
        if n > (self.len - self.pos) / 8 {
            return Err(Error::Truncated);
        }
        (0..n).map(|_| self.take(8).map(|n| n as u8)).collect()
    }
    fn count(&mut self, bits: usize, min_bits: usize) -> Result<usize, Error> {
        let count = self.take(bits)? as usize;
        if count > MAX_ITEMS {
            return Err(Error::BudgetExceeded);
        }
        if count.saturating_mul(min_bits) > self.len - self.pos {
            return Err(Error::Truncated);
        }
        Ok(count)
    }
    fn boolean(&mut self) -> Result<bool, Error> {
        match self.take(32)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::InvalidValue),
        }
    }
    fn present(&mut self) -> Result<(), Error> {
        if self.take(1)? == 1 {
            Ok(())
        } else {
            Err(Error::Unsupported)
        }
    }
    pub(crate) fn string(&mut self) -> Result<String, Error> {
        let n = self.take(32)? as i32;
        if n == 0 {
            return Ok(String::new());
        }
        let count = n.unsigned_abs() as usize;
        if count > 4096 {
            return Err(Error::BudgetExceeded);
        }
        if n < 0 {
            let values: Vec<u16> = (0..count)
                .map(|_| self.take(16).map(|v| v as u16))
                .collect::<Result<_, _>>()?;
            if values.last() != Some(&0) {
                return Err(Error::InvalidValue);
            }
            String::from_utf16(&values[..count - 1]).map_err(|_| Error::InvalidValue)
        } else {
            let values = self.bytes(count)?;
            if values.last() != Some(&0) || !values.is_ascii() {
                return Err(Error::Unsupported);
            }
            String::from_utf8(values[..count - 1].to_vec()).map_err(|_| Error::InvalidValue)
        }
    }
    pub(crate) fn packed(&mut self) -> Result<u32, Error> {
        let mut out = 0;
        for i in 0..5 {
            let byte = self.take(8)? as u32;
            if i == 4 && byte >> 1 > 15 {
                return Err(Error::InvalidValue);
            }
            out |= (byte >> 1) << (7 * i);
            if byte & 1 == 0 {
                return Ok(out);
            }
        }
        Err(Error::InvalidValue)
    }
    pub(crate) fn name(&mut self) -> Result<Name, Error> {
        if self.take(1)? == 1 {
            Ok(Name::Hardcoded(self.packed()?))
        } else {
            Ok(Name::Text {
                text: self.string()?,
                number: self.take(32)? as u32,
            })
        }
    }
    fn actor(&mut self) -> Result<ActorRef, Error> {
        let flags = self.take(3)? as u8;
        let mut fields = Vec::new();
        let name = if flags & 1 != 0 {
            Some(self.name()?)
        } else {
            for offset in [0, 4, 8, 12, 20, 24, 16] {
                fields.push((offset, self.take(32)? as u32));
            }
            None
        };
        if flags & 2 != 0 {
            fields.push((36, self.take(32)? as u32));
        }
        fields.sort_unstable();
        Ok(ActorRef {
            flags,
            name,
            fields,
        })
    }
    fn finish(self) -> Result<(), Error> {
        if self.pos == self.len {
            Ok(())
        } else {
            Err(Error::TrailingBits)
        }
    }
}

struct Wrapper {
    tag: u8,
    data: Vec<u8>,
}
impl Wrapper {
    fn u32(&self, tag: u8) -> Result<u32, Error> {
        if self.tag != tag {
            return Err(Error::Unsupported);
        }
        Ok(u32::from_le_bytes(
            self.data
                .as_slice()
                .try_into()
                .map_err(|_| Error::InvalidValue)?,
        ))
    }
    fn u64(&self, tag: u8) -> Result<u64, Error> {
        if self.tag != tag {
            return Err(Error::Unsupported);
        }
        Ok(u64::from_le_bytes(
            self.data
                .as_slice()
                .try_into()
                .map_err(|_| Error::InvalidValue)?,
        ))
    }
}

fn wrappers(r: &mut Bits<'_>) -> Result<Vec<Wrapper>, Error> {
    let n = r.count(32, 40)?;
    (0..n)
        .map(|_| {
            let tag = r.take(8)? as u8;
            if (36..=38).contains(&tag) {
                r.name()?;
            }
            if tag == 35 || tag == 37 {
                return Err(Error::Unsupported);
            }
            let len = r.take(32)? as usize;
            if len > 1024 * 1024 {
                return Err(Error::BudgetExceeded);
            }
            Ok(Wrapper {
                tag,
                data: r.bytes(len)?,
            })
        })
        .collect()
}

fn compact(r: &mut Bits<'_>) -> Result<Option<u32>, Error> {
    let flags = r.take(4)?;
    if flags & 2 != 0 {
        r.take(16)?;
    }
    let effect = if flags & 4 != 0 {
        Some(r.take(32)? as u32)
    } else {
        None
    };
    if flags & 8 != 0 {
        r.take(16)?;
    }
    Ok(effect)
}

fn nested_map(r: &mut Bits<'_>) -> Result<(), Error> {
    let n = r.count(32, 32)?;
    for _ in 0..n {
        r.take(32)?;
    }
    for _ in 0..n {
        r.take(32)?;
        let children = r.count(32, 8)?;
        r.bytes(children)?;
        for _ in 0..children {
            let leaves = r.count(32, 40)?;
            r.bytes(leaves)?;
            for _ in 0..leaves {
                r.take(32)?;
            }
        }
    }
    Ok(())
}

fn netfight(r: &mut Bits<'_>, message: i64) -> Result<RequestTarget, Error> {
    let version = r.take(8)?;
    if !matches!(version, 6 | 7) {
        return Err(Error::Unsupported);
    }
    let values = wrappers(r)?;
    wrappers(r)?; // Explicitly bounded opaque diagnostic wrappers; not fake decoded fields.
    if r.take(64)? as i64 != message {
        return Err(Error::InvalidValue);
    }
    let source = r.actor()?;
    let target = r.actor()?;
    let effect_index = compact(r)?;
    let count = r.count(32, 64)?;
    for _ in 0..count {
        r.take(64)?;
    }
    nested_map(r)?;
    let count = r.count(32, 4)?;
    for _ in 0..count {
        compact(r)?;
    }
    nested_map(r)?;
    let branch = r.take(4)?;
    match branch {
        1 => {
            for _ in 0..12 {
                r.take(32)?;
            }
        }
        2 => {
            for _ in 0..4 {
                r.take(32)?;
            }
        }
        _ => {}
    }
    // v7 expands this opaque flag region by two bits. It is framing, not
    // evidence of critical hits or damage categories.
    r.take(if version == 7 {
        V7_REQUEST_FLAGS_BITS
    } else {
        V6_REQUEST_FLAGS_BITS
    })?;
    if r.boolean()? {
        r.string()?;
    }
    for _ in 0..6 {
        r.boolean()?;
    }
    r.take(2)?;
    r.take(1)?;
    let profile = [
        (17, 24),
        (12, 4),
        (12, 4),
        (12, 4),
        (13, 8),
        (12, 4),
        (12, 4),
        (8, 1),
        (1, 4),
        (1, 4),
        (12, 4),
    ];
    let health = branch == 1
        && values.len() == profile.len()
        && values
            .iter()
            .zip(profile)
            .all(|(v, (tag, len))| v.tag == tag && v.data.len() == len);
    let float = |i: usize| -> Option<u32> {
        let raw = values.get(i)?.u32(12).ok()?;
        f32::from_bits(raw).is_finite().then_some(raw)
    };
    Ok(RequestTarget {
        source,
        target,
        effect_index,
        hp_before_bits: health
            .then(|| float(2))
            .flatten()
            .filter(|b| f32::from_bits(*b) >= 0.0),
        max_hp_bits: health
            .then(|| float(3))
            .flatten()
            .filter(|b| f32::from_bits(*b) > 0.0),
        calculated_damage_bits: health.then(|| float(1)).flatten(),
    })
}

pub fn decode_request(data: &[u8], bits: usize, channel: u32) -> Result<Request, Error> {
    let mut r = Bits::new(data, bits)?;
    r.present()?;
    let header = wrappers(&mut r)?;
    if header.len() != 4 {
        return Err(Error::Unsupported);
    }
    let message = header[1].u64(7)? as i64;
    let timestamp_bits = header[3].u64(13)?;
    r.present()?;
    let count = r.count(16, 8)?;
    let targets = (0..count)
        .map(|_| netfight(&mut r, message))
        .collect::<Result<_, _>>()?;
    r.finish()?;
    Ok(Request {
        key: MessageKey {
            channel,
            message,
            timestamp_bits,
        },
        targets,
    })
}

pub fn decode_settlement(data: &[u8], bits: usize, channel: u32) -> Result<Settlement, Error> {
    decode_settlement_format(data, bits, channel, false)
}

pub(super) fn decode_settlement_v7(
    data: &[u8],
    bits: usize,
    channel: u32,
) -> Result<Settlement, Error> {
    decode_settlement_format(data, bits, channel, true)
}

fn decode_settlement_format(
    data: &[u8],
    bits: usize,
    channel: u32,
    v7: bool,
) -> Result<Settlement, Error> {
    let mut r = Bits::new(data, bits)?;
    r.present()?;
    let message = r.take(64)? as i64;
    let source = r.actor()?;
    let count = r.count(16, 3)?;
    let mut targets = Vec::new();
    for _ in 0..count {
        let target = r.actor()?;
        let current_hp_bits = r.take(32)? as u32;
        let dead_state = r.take(32)? as i32;
        let shield_damage_bits = r.take(32)? as u32;
        let lock_target = r.take(32)? as i32;
        if !f32::from_bits(current_hp_bits).is_finite()
            || !f32::from_bits(shield_damage_bits).is_finite()
        {
            return Err(Error::InvalidValue);
        }
        // The v7 target observed in the qualified capture prefixes its flat
        // damage wrappers with this marker. Other marker values need wire evidence.
        if v7 && r.take(32)? != V7_DAMAGE_WRAPPERS_MARKER {
            return Err(Error::Unsupported);
        }
        let raw = wrappers(&mut r)?;
        let stride = if v7 { 3 } else { 2 };
        if !raw.len().is_multiple_of(stride) {
            return Err(Error::Unsupported);
        }
        let mut components = Vec::new();
        for values in raw.chunks_exact(stride) {
            let delta = values[0].u32(6)? as i32;
            if delta >= 0 {
                return Err(Error::InvalidValue);
            }
            let damage = delta.checked_neg().ok_or(Error::InvalidValue)?;
            let display_type = values[1].u32(31)? as i32;
            if v7 {
                // UETools test-server's CombatSettlementFormat confirms this
                // third enum is the text color, independent of the damage category.
                values[2].u32(31)?;
            }
            components.push(Component {
                damage,
                display_type,
            });
        }
        targets.push(SettledTarget {
            target,
            current_hp_bits,
            dead_state,
            shield_damage_bits,
            lock_target,
            components,
        });
    }
    // FClientReplicatedTargetDataContainer: extra-damage array first,
    // recovery array second. Extra-damage records are not yet qualified on wire.
    if r.count(16, 1)? != 0 {
        return Err(Error::UnsupportedSettlementExtras);
    }
    let recovery_count = r.count(16, 36)?;
    let mut recoveries = Vec::with_capacity(recovery_count);
    for _ in 0..recovery_count {
        let target = r.actor()?;
        let current_hp_bits = r.take(32)? as u32;
        let hp = f32::from_bits(current_hp_bits);
        if !hp.is_finite() || hp < 0.0 {
            return Err(Error::InvalidValue);
        }
        recoveries.push(RecoveredTarget {
            target,
            current_hp_bits,
        });
    }
    let timestamp_bits = r.take(64)?;
    r.take(3)?;
    r.name()?;
    r.finish()?;
    Ok(Settlement {
        recoveries,
        key: MessageKey {
            channel,
            message,
            timestamp_bits,
        },
        source,
        targets,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    #[derive(Default)]
    struct Writer {
        bytes: Vec<u8>,
        bits: usize,
    }
    impl Writer {
        fn put(&mut self, value: u64, count: usize) {
            self.bytes.resize((self.bits + count).div_ceil(8), 0);
            for i in 0..count {
                self.bytes[(self.bits + i) / 8] |=
                    (((value >> i) & 1) as u8) << ((self.bits + i) % 8);
            }
            self.bits += count;
        }
    }
    impl Writer {
        fn wrapper(&mut self, tag: u8, value: u64, bytes: usize) {
            self.put(u64::from(tag), 8);
            self.put(bytes as u64, 32);
            self.put(value, bytes * 8);
        }

        fn actor(&mut self) {
            self.put(0, 3);
            for word in [1, 2, 3, 4, 0, 0, 0] {
                self.put(word, 32);
            }
        }
    }

    #[test]
    fn v6_and_v7_damage_exchange_preserves_amounts_and_display_categories() {
        for version in [6, 7] {
            let mut request = Writer::default();
            request.put(1, 1);
            request.put(4, 32);
            for (tag, value, bytes) in [(31, 0, 4), (7, 7, 8), (6, 0, 4), (13, 123, 8)] {
                request.wrapper(tag, value, bytes);
            }
            request.put(1, 1);
            request.put(1, 16);
            request.put(version, 8);
            request.put(0, 32); // calculation wrappers
            request.put(0, 32); // diagnostic wrappers
            request.put(7, 64);
            request.actor();
            request.actor();
            request.put(4, 4); // compact effect is present
            request.put(17, 32);
            for _ in 0..4 {
                request.put(0, 32); // auxiliary arrays and maps
            }
            request.put(2, 4);
            for _ in 0..4 {
                request.put(0, 32); // branch 2 calculation fields
            }
            request.put(0, if version == 7 { 7 } else { 5 });
            request.put(0, 32); // optional string is absent
            for value in [1, 0, 1, 0, 0, 1] {
                request.put(value, 32);
            }
            request.put(0, 3);
            let decoded_request = decode_request(&request.bytes, request.bits, 3).unwrap();
            assert_eq!(decoded_request.targets[0].effect_index, Some(17));
            assert!(decode_request(&request.bytes, request.bits - 1, 3).is_err());

            let mut settlement = Writer::default();
            settlement.put(1, 1);
            settlement.put(7, 64);
            settlement.actor();
            settlement.put(1, 16);
            settlement.actor();
            settlement.put(u64::from(300f32.to_bits()), 32);
            for _ in 0..3 {
                settlement.put(0, 32); // death, shield damage, lock target
            }
            if version == 7 {
                settlement.put(1, 32); // v7 damage wrapper marker
            }
            settlement.put(if version == 7 { 6 } else { 4 }, 32);
            for (damage, display, color) in [(200, 22, 4), (30, 0, 8)] {
                settlement.wrapper(6, u64::from((-damage) as u32), 4);
                settlement.wrapper(31, display, 4);
                if version == 7 {
                    settlement.wrapper(31, color, 4);
                }
            }
            settlement.put(0, 16); // extra damage
            settlement.put(0, 16); // recoveries
            settlement.put(123, 64);
            settlement.put(0, 3);
            settlement.put(1, 1);
            settlement.put(0, 8); // hardcoded reaction name
            let decode = if version == 7 {
                decode_settlement_v7
            } else {
                decode_settlement
            };
            let decoded = decode(&settlement.bytes, settlement.bits, 3).unwrap();
            assert_eq!(decoded.key, decoded_request.key);
            assert_eq!(decoded.source, decoded_request.targets[0].source);
            assert_eq!(decoded.targets[0].target, decoded_request.targets[0].target);
            assert_eq!(
                decoded.targets[0].components,
                vec![
                    Component {
                        damage: 200,
                        display_type: 22
                    },
                    Component {
                        damage: 30,
                        display_type: 0
                    },
                ],
            );
            assert_eq!(
                decode(&settlement.bytes, settlement.bits - 1, 3),
                Err(Error::Truncated)
            );
        }
    }

    pub(crate) fn recovery_rpc(extra_count: u16, recovery_count: u16, hp: f32) -> (Vec<u8>, usize) {
        let mut w = Writer::default();
        w.put(1, 1);
        w.put(145, 64);
        w.put(1, 3);
        w.put(1, 1);
        w.put(0, 8); // source FName
        w.put(0, 16);
        w.put(u64::from(extra_count), 16);
        w.put(u64::from(recovery_count), 16);
        for _ in 0..recovery_count.min(2) {
            w.put(0, 3);
            for value in [1, 2, 3, 4, 0, 0, 0] {
                w.put(value, 32);
            }
            w.put(u64::from(hp.to_bits()), 32);
        }
        w.put(123, 64);
        w.put(0, 3);
        w.put(1, 1);
        w.put(0, 8);
        (w.bytes, w.bits)
    }
    #[test]
    fn recovery_array_is_bounded_finite_and_fully_consumed_without_damage() {
        let (bytes, bits) = recovery_rpc(0, 1, 800.0);
        let r = decode_settlement(&bytes, bits, 3).unwrap();
        assert!(r.targets.is_empty());
        assert_eq!(r.recoveries.len(), 1);
        assert_eq!(r.recoveries[0].current_hp_bits, 800f32.to_bits());
        assert_eq!(r.key.message, 145);
        assert_eq!(r.key.timestamp_bits, 123);
        assert_eq!(
            decode_settlement(&bytes, bits - 1, 3),
            Err(Error::Truncated)
        );
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(
            decode_settlement(&trailing, bits + 1, 3),
            Err(Error::TrailingBits)
        );
        for hp in [f32::NAN, f32::INFINITY, -1.0] {
            let (b, n) = recovery_rpc(0, 1, hp);
            assert_eq!(decode_settlement(&b, n, 3), Err(Error::InvalidValue));
        }
        let (b, n) = recovery_rpc(0, 1025, 1.0);
        assert_eq!(decode_settlement(&b, n, 3), Err(Error::BudgetExceeded));
        let (b, n) = recovery_rpc(1, 1, 1.0);
        assert_eq!(
            decode_settlement(&b, n, 3),
            Err(Error::UnsupportedSettlementExtras)
        );
    }
}
