//! Explicit-profile UE packet/Bunch/content-block framing for the new parser.
//! A profile is an input, never discovered by choosing whichever parse succeeds.
//! Caller owns connection/generation identity; one decoder cannot mix connections.
use super::wire::Bits;
use super::{Error, Name, Request, Settlement, decode_request, decode_settlement};
use serde::{Deserialize, Serialize};

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
    Request(Request),
    Settlement(Settlement),
}

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
        })
    }
    pub fn has_incomplete_fragments(&self) -> bool {
        self.pending.iter().any(Option::is_some)
    }
    pub fn clear(&mut self) {
        self.pending = [None, None];
    }

    pub fn datagram(&mut self, data: &[u8], inbound: bool) -> Result<Vec<Rpc>, Error> {
        if data.len() > 65535 {
            return Err(Error::BudgetExceeded);
        }
        let outer_len = termination(data)?;
        let mut outer = Bits::new(data, outer_len)?;
        if outer.take(6)? != u64::from(self.profile.component_prefix) {
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
        for b in bunches {
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

    fn message(
        &self,
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
            // Actor reference only; spawning a new dynamic actor needs its own
            // qualified spawn profile and is not silently skipped here.
            if guid != 0 && guid & 1 == 0 {
                return Err(Error::Unsupported);
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
                    result.push(Rpc::Settlement(decode_settlement(
                        &raw,
                        count,
                        self.profile.channel,
                    )?));
                }
            }
        }
        Ok(result)
    }
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
