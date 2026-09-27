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

/// Stable warning code shared by capture and its read models. No local paths
/// or endpoint details may cross the frontend boundary.
pub const PROFILE_MISSING: &str = "exact_profile_missing_damage_unavailable_no_legacy_fallback";
pub const CONFIG_FILE_NAME: &str = "exact-packet.json";

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
    pub warning: Option<&'static str>,
}

fn json_file(path: &Path, limit: u64) -> Result<serde_json::Value, &'static str> {
    let file = fs::File::open(path).map_err(|_| "exact_input_open_failed")?;
    let meta = file.metadata().map_err(|_| "exact_input_metadata_failed")?;
    if !meta.is_file() {
        return Err("exact_input_not_a_file");
    }
    if meta.len() > limit {
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
        let override_path = std::env::var_os("NTE_EXACT_PACKET_CONFIG");
        Self::from_paths(
            override_path.as_deref().map(Path::new),
            &crate::storage::paths::software_dir(),
        )
    }

    // Separate path resolution from process environment so tests never race by
    // changing environment variables. An explicit override never falls back.
    fn from_paths(
        override_path: Option<&Path>,
        software_dir: &Path,
    ) -> Result<Option<Self>, &'static str> {
        let default_path = software_dir.join(CONFIG_FILE_NAME);
        let path = override_path.unwrap_or(&default_path);
        match fs::metadata(path) {
            Err(error)
                if override_path.is_none() && error.kind() == std::io::ErrorKind::NotFound =>
            {
                return Ok(None);
            }
            Err(_) => return Err("exact_input_metadata_failed"),
            Ok(metadata) if !metadata.is_file() => return Err("exact_input_not_a_file"),
            Ok(_) => {}
        }
        let mut config: Config = serde_json::from_value(json_file(path, 64 * 1024)?)
            .map_err(|_| "exact_profile_invalid")?;
        // Portable configurations resolve their catalog beside the config,
        // not against Explorer's (or a shortcut's) working directory.
        if Path::new(&config.catalog_path).is_relative() {
            config.catalog_path = path
                .parent()
                .unwrap_or(Path::new("."))
                .join(&config.catalog_path)
                .to_str()
                .ok_or("exact_catalog_path_invalid")?
                .to_owned();
        }
        Self::from_config(config).map(Some)
    }
    pub fn from_config(config: Config) -> Result<Self, &'static str> {
        let document = json_file(Path::new(&config.catalog_path), 128 * 1024 * 1024)?;
        let catalog = catalog_from_document(&document)?;
        let decoder = Decoder::new(config.rpc.clone()).map_err(|_| "exact_profile_invalid")?;
        let generation = new_generation()?;
        Ok(Self {
            config,
            decoder,
            ledger: Ledger::new(100_000, catalog),
            generation,
            failed: false,
            last_settlement_components: 0,
            warning: None,
        })
    }
}

pub(super) fn catalog_from_document(
    document: &serde_json::Value,
) -> Result<SkillCatalog, &'static str> {
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
        if skill["name"].as_str().is_some_and(|name| name.len() > 512) {
            return Err("exact_skill_name_budget");
        }
        let owners = skill["owners"]
            .as_array()
            .ok_or("exact_skill_owners_missing")?;
        if owners.len() > 256 {
            return Err("exact_skill_owners_budget");
        }
        let owner_names = owners
            .iter()
            .map(|o| {
                o["characterId"]
                    .as_str()
                    .filter(|id| id.len() <= 128)
                    .ok_or("exact_owner_invalid")
            })
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
    load_mechanics(&mut catalog)?;
    Ok(catalog)
}

fn load_mechanics(catalog: &mut SkillCatalog) -> Result<(), &'static str> {
    use crate::storage::resource::read_resource_text_bounded;
    let resource = |path: &str| -> Result<serde_json::Value, &'static str> {
        let text = read_resource_text_bounded(Path::new(path), 16 * 1024 * 1024)
            .map_err(|_| "exact_mechanic_resource_unavailable")?;
        serde_json::from_str(&text).map_err(|_| "exact_mechanic_resource_invalid")
    };
    let mapping = resource(crate::engine::parser::GAMEPLAY_EFFECT_MAPPING_PATH)?;
    let semantics = resource(crate::engine::parser::GAMEPLAY_EFFECT_SEMANTICS_PATH)?;
    let rows = mapping[0]["Rows"]
        .as_object()
        .ok_or("exact_mechanic_mapping_invalid")?;
    let labels = semantics["effects"]
        .as_object()
        .ok_or("exact_mechanic_semantics_invalid")?;
    if rows.len() > 16384 || labels.len() > 16384 {
        return Err("exact_mechanic_budget");
    }
    let mut unique = HashMap::<u32, Option<super::EffectMechanic>>::new();
    for (name, value) in rows {
        if name.len() > 512 {
            return Err("exact_mechanic_name_budget");
        }
        let index = value["UniqueIndex"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or("exact_mechanic_index_invalid")?;
        let label = labels.get(name);
        let display = label
            .and_then(|s| s["damage_name_zh"].as_str())
            .filter(|_| label.is_some_and(|s| s["show_parent_ability"].as_bool() == Some(false)));
        if display.is_some_and(|s| s.len() > 512) {
            return Err("exact_mechanic_label_budget");
        }
        let percent = label
            .and_then(|s| s["max_hp_reduction_percent"].as_u64())
            .unwrap_or(0);
        let owner = label
            .and_then(|s| s["owner_character_id"].as_u64())
            .and_then(|n| u32::try_from(n).ok());
        if percent > 10000 || (percent > 0 && owner.is_none()) {
            return Err("exact_mechanic_rule_invalid");
        }
        let mechanic = super::EffectMechanic {
            unbalance_label: label.and_then(|s| s["attack_type"].as_str())
                == Some(crate::engine::model::UNBALANCE_ATTACK_TYPE),
            effect_name: name.clone(),
            display_name: display.map(str::to_owned),
            max_hp_reduction_percent: percent as u32,
            owner,
        };
        unique
            .entry(index)
            .and_modify(|entry| *entry = None)
            .or_insert(Some(mechanic));
    }
    catalog.mechanics = unique
        .into_iter()
        .filter_map(|(key, value)| value.map(|v| (key, v)))
        .collect();
    Ok(())
}

pub(super) fn new_generation() -> Result<String, &'static str> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| "exact_generation_clock_invalid")?
        .as_nanos();
    Ok(format!(
        "{}-{now}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

impl Runtime {
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
        self.warning = None;
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
                Rpc::Inventory { .. } => continue,
                Rpc::UnsupportedSettlementExtras => {
                    self.ledger.invalidate_hp_continuity();
                    self.warning = Some(super::automatic::UNSUPPORTED_SETTLEMENT);
                    continue;
                }
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
            for update in self.ledger.take_hp_updates() {
                result.push(project(
                    update,
                    &self.generation,
                    "configured-flow",
                    time,
                    characters,
                    include_incoming,
                ));
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mechanic_labels_cover_non_skill_table_effects_without_inventing_skill_ownership() {
        let mut c = SkillCatalog::default();
        load_mechanics(&mut c).unwrap();
        assert_eq!(
            c.mechanics[&6094].effect_name,
            "GE_Reaction_4_new_1042_Damage"
        );
        assert_eq!(
            c.mechanics[&6094].display_name.as_deref(),
            Some("鸫歌·黯星强化结算")
        );
        assert_eq!(c.mechanics[&2537].display_name.as_deref(), Some("承轨反击"));
        assert_eq!(c.mechanics[&4503].display_name.as_deref(), Some("浊燃"));
        for (index, name) in [(749, "普通倾陷伤害"), (2949, "达芙蒂尔·额外倾陷伤害")]
        {
            assert_eq!(c.mechanics[&index].display_name.as_deref(), Some(name));
            assert!(c.mechanics[&index].unbalance_label);
        }
        assert_eq!(
            c.mechanics[&3311].display_name.as_deref(),
            Some("漆黑青春妄想·黑之书")
        );
        assert!(!c.mechanics[&3311].unbalance_label);
        assert_eq!(c.mechanics[&599].max_hp_reduction_percent, 200);
        assert_eq!(c.mechanics[&599].owner, Some(1004));
        for (index, label) in [
            (1349, "墨菲克斯·弹丸回击"),
            (3241, "番茄酱盛宴·失谐强化追加伤害"),
            (6114, "失乐鸟·协同攻击"),
            (4457, "桀桀熊·act08第1段"),
            (1324, "墨菲克斯·act10攻击"),
        ] {
            assert_eq!(c.mechanics[&index].display_name.as_deref(), Some(label));
            assert_eq!(c.mechanics[&index].max_hp_reduction_percent, 0);
        }
        assert!(
            c.effects.is_empty(),
            "presentation labels must not manufacture GA roots"
        );
    }
    #[test]
    fn current_resource_catalog_loads_new_character_skills_without_guessing_owners() {
        let catalog_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("res/data/skills/skill_attribution_catalog.json");
        let runtime = Runtime::from_config(Config {
            rpc: Profile {
                component_prefix: 20,
                channel: 3,
                field_upper_exclusive: 219,
                request_index: 100,
                settlement_index: 142,
            },
            local_ip: Ipv4Addr::LOCALHOST,
            local_port: 12345,
            server_ip: Ipv4Addr::new(192, 0, 2, 1),
            server_port: 23456,
            catalog_path: catalog_path.to_str().unwrap().to_owned(),
        })
        .unwrap();
        for (key, owner) in [("GA_BlackBird_Melee", 1042), ("GA_Akane_Skill", 1057)] {
            assert!(
                runtime
                    .ledger
                    .catalog
                    .effects
                    .values()
                    .flatten()
                    .any(|skill| {
                        skill.key == key && skill.owners.contains(&owner) && skill.name.is_some()
                    })
            );
        }
        let document = json_file(&catalog_path, 128 * 1024 * 1024).unwrap();
        for effect in document["damageEffects"].as_object().unwrap().values() {
            if let Some(index) = effect["effectIndex"].as_u64() {
                if effect["statisticalSkillKey"].is_null() {
                    assert!(
                        runtime
                            .ledger
                            .catalog
                            .unresolved_effects
                            .contains(&(index as u32))
                    );
                } else {
                    assert!(runtime.ledger.catalog.effects.contains_key(&(index as u32)));
                }
            }
        }
    }

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "nte-exact-config-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn configure(&self) {
            fs::write(self.0.join("catalog.json"), r#"{"damageEffects":{}}"#).unwrap();
            fs::write(self.0.join(CONFIG_FILE_NAME), r#"{
                "rpc":{"component_prefix":20,"channel":3,"field_upper_exclusive":219,"request_index":100,"settlement_index":142},
                "local_ip":"127.0.0.1","local_port":12345,"server_ip":"192.0.2.1","server_port":23456,
                "catalog_path":"catalog.json"
            }"#).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn portable_profile_works_without_environment_and_resolves_catalog_beside_config() {
        let f = Fixture::new();
        f.configure();
        let first = Runtime::from_paths(None, &f.0).unwrap().unwrap();
        let second = Runtime::from_paths(None, &f.0).unwrap().unwrap();
        assert_eq!(
            Path::new(&first.config.catalog_path),
            f.0.join("catalog.json")
        );
        assert_ne!(first.generation, second.generation);
        assert_eq!(first.config.local_port, 12345);
    }

    #[test]
    fn absent_default_is_unavailable_but_explicit_missing_override_is_an_error() {
        let f = Fixture::new();
        assert!(Runtime::from_paths(None, &f.0).unwrap().is_none());
        f.configure();
        assert!(matches!(
            Runtime::from_paths(Some(&f.0.join("missing.json")), &f.0),
            Err("exact_input_metadata_failed")
        ));
    }

    #[test]
    fn malformed_or_oversized_profile_does_not_fall_back_and_retry_is_usable() {
        let f = Fixture::new();
        f.configure();
        let path = f.0.join(CONFIG_FILE_NAME);
        fs::write(&path, b"{").unwrap();
        assert!(matches!(
            Runtime::from_paths(None, &f.0),
            Err("exact_input_json_invalid")
        ));
        fs::write(&path, vec![b' '; 65537]).unwrap();
        assert!(matches!(
            Runtime::from_paths(None, &f.0),
            Err("exact_input_budget_exceeded")
        ));
        f.configure();
        assert!(Runtime::from_paths(None, &f.0).unwrap().is_some());
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(matches!(
            Runtime::from_paths(None, &f.0),
            Err("exact_input_not_a_file")
        ));
    }

    #[test]
    fn explicit_override_wins_and_never_reuses_default_endpoints() {
        let f = Fixture::new();
        f.configure();
        let other = Fixture::new();
        other.configure();
        let path = other.0.join(CONFIG_FILE_NAME);
        let mut config: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        config["local_port"] = 43210.into();
        fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
        let mut runtime = Runtime::from_paths(Some(&path), &f.0).unwrap().unwrap();
        assert_eq!(runtime.config.local_port, 43210);
        assert!(
            runtime
                .datagram(
                    Ipv4Addr::LOCALHOST,
                    12345,
                    Ipv4Addr::new(192, 0, 2, 1),
                    23456,
                    &[],
                    Some(1.0),
                    &HashMap::new(),
                    true
                )
                .unwrap()
                .is_empty()
        );
        assert!(!runtime.failed);
    }
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
            warning: None,
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
