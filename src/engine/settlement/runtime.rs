//! Shared live/import adapter. Profiles are explicit, never guessed from a
//! successful payload parse; missing configuration cannot enable legacy damage.
use super::{
    Ledger, Skill, SkillCatalog,
    application::{Projection, project},
    transport::{Decoder, Profile, Rpc},
};
use crate::engine::model::CharacterInfo;
use serde::Deserialize;
use std::{
    collections::HashMap,
    fs,
    io::Read,
    net::Ipv4Addr,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

#[derive(Deserialize)]
pub struct Config {
    pub rpc: Profile,
    pub local_ip: Ipv4Addr,
    pub local_port: u16,
    pub server_ip: Ipv4Addr,
    pub server_port: u16,
    pub catalog_path: String,
}

pub struct Runtime {
    config: Config,
    decoder: Decoder,
    ledger: Ledger,
    generation: String,
    failed: bool,
    pub last_settlement_components: usize,
}

fn json_file(path: &Path, limit: u64) -> Result<serde_json::Value, &'static str> {
    let file = fs::File::open(path).map_err(|_| "exact_input_open_failed")?;
    let meta = file.metadata().map_err(|_| "exact_input_metadata_failed")?;
    if !meta.is_file() || meta.len() > limit {
        return Err("exact_input_budget_exceeded");
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "exact_input_read_failed")?;
    if bytes.len() as u64 > limit {
        return Err("exact_input_budget_exceeded");
    }
    serde_json::from_slice(&bytes).map_err(|_| "exact_input_json_invalid")
}

impl Runtime {
    pub fn from_environment() -> Result<Option<Self>, &'static str> {
        let Some(path) = std::env::var_os("NTE_EXACT_PACKET_CONFIG") else {
            return Ok(None);
        };
        let config: Config = serde_json::from_value(json_file(Path::new(&path), 64 * 1024)?)
            .map_err(|_| "exact_profile_invalid")?;
        Self::from_config(config).map(Some)
    }
    pub fn from_config(config: Config) -> Result<Self, &'static str> {
        let document = json_file(Path::new(&config.catalog_path), 128 * 1024 * 1024)?;
        let effects = document["damageEffects"]
            .as_object()
            .ok_or("exact_effect_catalog_missing")?;
        if effects.len() > 16384 {
            return Err("exact_effect_catalog_budget");
        }
        let mut catalog = SkillCatalog::default();
        for effect in effects.values() {
            let Some(index) = effect["effectIndex"]
                .as_u64()
                .and_then(|i| u32::try_from(i).ok())
            else {
                continue;
            };
            let Some(root) = effect["statisticalSkillKey"].as_str() else {
                catalog.unresolved_effects.insert(index);
                continue;
            };
            if root.len() > 512 {
                return Err("exact_skill_key_budget");
            }
            let skill = &document["skills"][root];
            let owners = skill["owners"]
                .as_array()
                .ok_or("exact_skill_owners_missing")?;
            if owners.len() > 256 {
                return Err("exact_skill_owners_budget");
            }
            let owner_names = owners
                .iter()
                .map(|o| o["characterId"].as_str().ok_or("exact_owner_invalid"))
                .collect::<Result<Vec<_>, _>>()?;
            // Named template owners are valid catalog rows but cannot match a
            // numeric wire character ID. Never strip prefixes to invent an ID.
            let owners = owner_names
                .into_iter()
                .filter_map(|s| s.parse().ok())
                .collect();
            catalog.effects.entry(index).or_default().push(Skill {
                key: root.into(),
                name: skill["name"].as_str().map(str::to_owned),
                owners,
            });
        }
        let decoder = Decoder::new(config.rpc.clone()).map_err(|_| "exact_profile_invalid")?;
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|_| "exact_generation_clock_invalid")?
            .as_nanos();
        let generation = format!(
            "{}-{now}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        Ok(Self {
            config,
            decoder,
            ledger: Ledger::new(100_000, catalog),
            generation,
            failed: false,
            last_settlement_components: 0,
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub fn datagram(
        &mut self,
        src: Ipv4Addr,
        sport: u16,
        dst: Ipv4Addr,
        dport: u16,
        payload: &[u8],
        time: Option<f64>,
        characters: &HashMap<u32, CharacterInfo>,
        include_incoming: bool,
    ) -> Result<Vec<Projection>, &'static str> {
        self.last_settlement_components = 0;
        if self.failed {
            return Ok(vec![]);
        }
        let c = &self.config;
        let inbound =
            (src, sport, dst, dport) == (c.server_ip, c.server_port, c.local_ip, c.local_port);
        let outbound =
            (dst, dport, src, sport) == (c.server_ip, c.server_port, c.local_ip, c.local_port);
        if !inbound && !outbound {
            return Ok(vec![]);
        }
        let Some(time) = time.filter(|t| t.is_finite()) else {
            self.failed = true;
            return Err("exact_capture_timestamp_unavailable");
        };
        let messages = match self.decoder.datagram(payload, inbound) {
            Ok(v) => v,
            Err(_) => {
                self.failed = true;
                return Err("exact_decode_failed_no_legacy_fallback");
            }
        };
        let mut result = Vec::new();
        for message in messages {
            let is_settlement = matches!(&message, Rpc::Settlement(_));
            let change = match message {
                Rpc::Request(r) => self.ledger.request(r),
                Rpc::Settlement(s) => self.ledger.settlement(s),
            };
            match change {
                Ok(Some(change)) => {
                    let projection = project(
                        change,
                        &self.generation,
                        "configured-flow",
                        time,
                        characters,
                        include_incoming,
                    );
                    if is_settlement {
                        self.last_settlement_components += projection.hits.len();
                    }
                    result.push(projection);
                }
                Ok(None) => {}
                Err(_) => {
                    self.failed = true;
                    return Err("exact_ledger_failed_no_legacy_fallback");
                }
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_capture_timestamp_fails_closed_instead_of_inventing_zero() {
        let profile = Profile {
            component_prefix: 20,
            channel: 3,
            field_upper_exclusive: 219,
            request_index: 100,
            settlement_index: 142,
        };
        let local = Ipv4Addr::LOCALHOST;
        let server = Ipv4Addr::new(192, 0, 2, 1);
        let config = Config {
            rpc: profile.clone(),
            local_ip: local,
            local_port: 12345,
            server_ip: server,
            server_port: 23456,
            catalog_path: String::new(),
        };
        let mut runtime = Runtime {
            config,
            decoder: Decoder::new(profile).unwrap(),
            ledger: Ledger::new(4, SkillCatalog::default()),
            generation: "synthetic".into(),
            failed: false,
            last_settlement_components: 0,
        };
        assert!(matches!(
            runtime.datagram(
                local,
                12345,
                server,
                23456,
                &[],
                None,
                &HashMap::new(),
                true
            ),
            Err("exact_capture_timestamp_unavailable")
        ));
        assert_eq!(runtime.last_settlement_components, 0);
        assert!(
            runtime
                .datagram(
                    local,
                    12345,
                    server,
                    23456,
                    &[],
                    Some(1.0),
                    &HashMap::new(),
                    true
                )
                .unwrap()
                .is_empty()
        );
    }
}
