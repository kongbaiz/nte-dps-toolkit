//! Native v6 RPC parameter grammar ported from the verified research decoder.
//! Unknown branches/extra arrays fail visibly; no arbitrary byte-pattern search.
use super::*;

const MAX_ITEMS: usize = 1024;
const MAX_RPC_BITS: usize = 8 * 1024 * 1024;

pub(super) struct Bits<'a> {
    pub(super) data: &'a [u8],
    pub(super) pos: usize,
    pub(super) len: usize,
}
impl<'a> Bits<'a> {
    pub(super) fn new(data: &'a [u8], len: usize) -> Result<Self, Error> {
        if len > MAX_RPC_BITS {
            return Err(Error::BudgetExceeded);
        }
        if len > data.len().saturating_mul(8) {
            return Err(Error::Truncated);
        }
        Ok(Self { data, pos: 0, len })
    }
    pub(super) fn take(&mut self, n: usize) -> Result<u64, Error> {
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
    pub(super) fn bytes(&mut self, n: usize) -> Result<Vec<u8>, Error> {
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
    pub(super) fn string(&mut self) -> Result<String, Error> {
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
    pub(super) fn packed(&mut self) -> Result<u32, Error> {
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
    pub(super) fn name(&mut self) -> Result<Name, Error> {
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
    if r.take(8)? != 6 {
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
    r.take(5)?;
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
        let raw = wrappers(&mut r)?;
        if raw.len() % 2 != 0 {
            return Err(Error::Unsupported);
        }
        let mut components = Vec::new();
        for pair in raw.as_chunks::<2>().0 {
            let delta = pair[0].u32(6)? as i32;
            if delta >= 0 {
                return Err(Error::InvalidValue);
            }
            let damage = delta.checked_neg().ok_or(Error::InvalidValue)?;
            let display_type = pair[1].u32(31)? as i32;
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
    // Do not silently skip an unimplemented nonempty recovery/extra-damage array.
    if r.take(16)? != 0 || r.take(16)? != 0 {
        return Err(Error::Unsupported);
    }
    let timestamp_bits = r.take(64)?;
    r.take(3)?;
    r.name()?;
    r.finish()?;
    Ok(Settlement {
        key: MessageKey {
            channel,
            message,
            timestamp_bits,
        },
        source,
        targets,
    })
}
