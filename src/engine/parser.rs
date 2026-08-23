use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::engine::model::{
    CharacterInfo, DamageWireEvent, EmptyCurtainItem, EnemyIdentity, EquipmentStat, Hit,
    HitCharacterSource, HitDirection, HtItemNetId,
};
use crate::engine::protocol::{BunchPacket, SequencedPacket};
use crate::storage::i18n::Language;
use crate::storage::resource::{read_resource_text, resource_exists, resource_file_path};

const RECORD_FIELD_COUNT: usize = 10;
const RECORD_PREFIX_FIELD_TYPES: [u8; 6] = [12, 12, 12, 13, 12, 12];
const RECORD_PREFIX_FIELD_LENGTHS: [usize; 6] = [4, 4, 4, 8, 4, 4];
const LEGACY_STATE_FIELD_TYPES: [u8; 3] = [6, 6, 6];
const LEGACY_STATE_FIELD_LENGTHS: [usize; 3] = [4, 4, 4];
const BOOL_ENUM_STATE_FIELD_TYPES: [u8; 3] = [8, 1, 1];
const BOOL_ENUM_STATE_FIELD_LENGTHS: [usize; 3] = [1, 4, 4];
const RECORD_TRAILING_FIELD_TYPE: u8 = 12;
const RECORD_TRAILING_FIELD_LENGTH: usize = 4;
const MAX_RECORD_FIELD_LENGTH: usize = 8;
const MIN_DAMAGE: f32 = 2.0;
// Damage and boss HP are serialized as f32. 999-night encounters can reach tens of
// billions, while a one-trillion bound still rejects implausible shifted-float matches.
const MAX_COMBAT_VALUE: f32 = 1_000_000_000_000.0;
const MAX_PLAUSIBLE_CURRENT_HP_UPDATE: f32 = 500_000.0;
const CURRENT_HP_PREFIX_LENGTH: usize = 16;
const BOSS_HP_PREFIX_LENGTH: usize = 36;
const BOSS_HP_PREFIX_HEAD: [u8; 8] = [0x06, 0x00, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00];
const CLIENT_FIGHT_TARGET_WIRE_BITS: usize = 227;
const CLIENT_FIGHT_TARGET_WIRE_BYTES: usize = CLIENT_FIGHT_TARGET_WIRE_BITS.div_ceil(8);
const COMPACT_CLIENT_FIGHT_IDENTITY_BYTES: usize = 28;
const COMPACT_CLIENT_FIGHT_RECORD_BYTES: usize = 52;
// Current SDK damage replication places the compact 28-byte
// `FCharacterForNet` target exactly 160 bytes after the decoded damage value.
const COMPACT_DAMAGE_RECORD_TARGET_WIRE_BIT_DELTA: usize = 1_280;
// Older bit-packed SDK builds used the 227-bit serializer at this displacement.
const BITPACKED_DAMAGE_RECORD_TARGET_WIRE_BIT_DELTA: usize = 1_277;
const CLIENT_FIGHT_DATA_ARRAY_HEADER_BITS: usize = 61;
const CLIENT_FIGHT_DATA_ELEMENT_BITS: usize = 531;
const CLIENT_FIGHT_DATA_HP_BIT_OFFSET: usize = 288;
const CLIENT_FIGHT_DATA_DEAD_STATE_BIT_OFFSET: usize = 320;
const CLIENT_FIGHT_DATA_WRAPPER_COUNT_BIT_OFFSET: usize = 416;
const CLIENT_FIGHT_DATA_FIRST_WRAPPER_TYPE_BIT_OFFSET: usize = 448;
const CLIENT_FIGHT_DATA_FIRST_WRAPPER_LENGTH_BIT_OFFSET: usize = 456;
const CLIENT_FIGHT_DATA_FIRST_WRAPPER_VALUE_BIT_OFFSET: usize = 488;
const CLIENT_FIGHT_DATA_SECOND_WRAPPER_TYPE_BIT_OFFSET: usize = 520;
const CLIENT_FIGHT_DATA_SECOND_WRAPPER_LENGTH_BIT_OFFSET: usize = 528;
const CLIENT_FIGHT_DATA_SECOND_WRAPPER_VALUE_BIT_OFFSET: usize = 560;
const SERVER_DAMAGE_WRAPPER_TYPE: u8 = 0x06;
const SERVER_DAMAGE_SECONDARY_WRAPPER_TYPE: u8 = 0x1f;
const MAX_CLIENT_FIGHT_DATA_ELEMENTS: usize = 64;
const CLIENT_DAMAGE_BOSS_PACKET_INFO_BITS: usize = 913;
const CLIENT_DAMAGE_BOSS_PAYLOAD_BITS: usize = 1_053;
const CLIENT_DAMAGE_BOSS_PREFIX_BYTE_OFFSET: usize = 29;
const CLIENT_DAMAGE_BOSS_BIT_SHIFT: u8 = 2;
const ACTIVE_GAMEPLAY_EFFECT_ANCHOR: &[u8] = b"FHTClientActiveGE";
const ACTIVE_GAMEPLAY_EFFECT_VALUE_OFFSET: usize = 5;
const ACTIVE_GAMEPLAY_EFFECT_MARKER: u32 = 12;
// Compact FHTClientActiveGE values omit the repeated property name and use
// either `0c,index,0b,state` or `04,index,state` before the shared trailer.
const COMPACT_GAMEPLAY_EFFECT_MARKER_WITH_FIELD: u8 = 12;
const COMPACT_GAMEPLAY_EFFECT_MARKER: u8 = 4;
const COMPACT_GAMEPLAY_EFFECT_FIELD: &[u8] = &[11, 0, 0, 0];
const COMPACT_GAMEPLAY_EFFECT_TRAILER: &[u8] = &[59, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0];
const BOOL_ENUM_COMPACT_GAMEPLAY_EFFECT_TRAILER: &[u8] =
    &[0, 124, 16, 0, 0, 0, 0, 0, 0, 0, 28, 32, 0, 0, 0];
const EQUIPMENT_SLOT_STATE_ANCHOR: &[u8] = b"\x06\0\0\0State\0";
const EMPTY_CURTAIN_CORE_SLOT_ANCHOR: &[u8] = b"FEquipmentSlotInfo";
const EMPTY_CURTAIN_GRID_SIDE: i32 = 7;
const EMPTY_CURTAIN_GRID_CELLS: usize = 49;
const MAX_TAGGED_PROPERTY_STRING_LENGTH: i32 = 256;
const MAX_INVENTORY_NAME_LENGTH: usize = 128;
const MAX_INVENTORY_EXTRA_JSON_LENGTH: usize = 16 * 1024;
const MAX_INVENTORY_STAT_VALUE: f32 = 1_000_000.0;
const MAX_INVENTORY_CHARACTER_LEVEL: i32 = 100;
const MAX_INVENTORY_CHARACTER_ARRAY_LENGTH: u16 = 64;
const MAX_EMPTY_CURTAIN_ITEMS: usize = 4096;

pub const EMPTY_CURTAIN_MAX_STAT_ROWS: usize = 6;

pub const CHARACTER_DATA_PATH: &str = "res/data/characters/characters.json";
pub const GAMEPLAY_EFFECT_MAPPING_PATH: &str = "res/data/skills/gameplay_effect_mapping.json";
pub const GAMEPLAY_EFFECT_SEMANTICS_PATH: &str = "res/data/skills/gameplay_effect_semantics.json";
pub const SKILL_DAMAGE_DATA_PATH: &str = "res/data/skills/skill_damage.json";
pub const ABILITY_TIPS_PATH: &str = "res/data/skills/ability_tips.json";
pub const EQUIPMENT_CATALOG_PATH: &str = "res/data/equipment/equipment.json";
pub const ENEMY_CATALOG_PATH: &str = "res/data/enemies/enemies.json";

#[derive(Clone, Debug, Default)]
pub struct EnemyCatalog {
    enemies: HashMap<u64, EnemyIdentity>,
}

impl EnemyCatalog {
    pub fn get(&self, config_hash: u64) -> Option<&EnemyIdentity> {
        self.enemies.get(&config_hash)
    }

    pub fn monster_ids(&self) -> Vec<String> {
        let mut monster_ids = self
            .enemies
            .values()
            .map(|enemy| enemy.monster_id.clone())
            .collect::<Vec<_>>();
        monster_ids.sort();
        monster_ids.dedup();
        monster_ids
    }
}

#[derive(Deserialize)]
struct EnemyCatalogDocument {
    version: u32,
    hash: String,
    enemies: Vec<EnemyCatalogEntry>,
}

#[derive(Deserialize)]
struct EnemyCatalogEntry {
    config_id: String,
    config_hash: String,
    monster_id: String,
    name_en: String,
    name_zh: String,
    name_ja: String,
}

// DT_SkillDamageData assigns Jin's time-stop katana hits to an unnamed internal
// ability, while the parent ability owns those GE rows and the localized name.
const ABILITY_DISPLAY_NAME_ALIASES: [(&str, &str); 1] =
    [("GA_Jin_UltraSkill_Melee", "GA_Jin_UltraSkill")];

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EquipmentKind {
    Module,
    Core,
}

#[derive(Clone, Debug, Deserialize)]
pub struct EquipmentItemDefinition {
    pub kind: EquipmentKind,
    pub name_zh: String,
    pub name_en: String,
    pub name_ja: String,
    pub quality: String,
    pub geometry: String,
    pub grid: Option<u32>,
    pub suit: Option<String>,
    pub icon: String,
    pub max_level: u32,
    pub main_count: usize,
    pub sub_count: usize,
}

impl EquipmentItemDefinition {
    pub fn name(&self, language: Language) -> &str {
        match language {
            Language::English => &self.name_en,
            Language::Japanese => &self.name_ja,
            Language::SimplifiedChinese => &self.name_zh,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct EquipmentAttributeDefinition {
    pub name_zh: String,
    pub name_en: String,
    pub name_ja: String,
    pub percent: bool,
}

impl EquipmentAttributeDefinition {
    pub fn name(&self, language: Language) -> &str {
        match language {
            Language::English => &self.name_en,
            Language::Japanese => &self.name_ja,
            Language::SimplifiedChinese => &self.name_zh,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct EquipmentSuitEffect {
    pub count: u32,
    pub text_zh: String,
    pub text_en: String,
    pub text_ja: String,
}

impl EquipmentSuitEffect {
    pub fn text(&self, language: Language) -> &str {
        match language {
            Language::English => &self.text_en,
            Language::Japanese => &self.text_ja,
            Language::SimplifiedChinese => &self.text_zh,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct EquipmentSuitDefinition {
    pub name_zh: String,
    pub name_en: String,
    pub name_ja: String,
    pub effects: Vec<EquipmentSuitEffect>,
}

impl EquipmentSuitDefinition {
    pub fn name(&self, language: Language) -> &str {
        match language {
            Language::English => &self.name_en,
            Language::Japanese => &self.name_ja,
            Language::SimplifiedChinese => &self.name_zh,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash)]
pub struct EquipmentGridCell {
    pub row: i32,
    pub column: i32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct EquipmentShapeDefinition {
    pub cells: Vec<EquipmentGridCell>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct EquipmentPlanModule {
    pub item_id: String,
    pub row: i32,
    pub column: i32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct EquipmentPlanDefinition {
    pub valid_cells: Vec<EquipmentGridCell>,
    pub recommended_core: String,
    pub recommended_modules: Vec<EquipmentPlanModule>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct EquipmentCatalog {
    version: u32,
    #[serde(default, rename = "sources")]
    _sources: serde_json::Value,
    pub items: HashMap<String, EquipmentItemDefinition>,
    pub attributes: HashMap<String, EquipmentAttributeDefinition>,
    pub curves: HashMap<String, Vec<[f32; 2]>>,
    pub suits: HashMap<String, EquipmentSuitDefinition>,
    pub shapes: HashMap<String, EquipmentShapeDefinition>,
    pub plans: HashMap<u32, EquipmentPlanDefinition>,
}

impl EquipmentCatalog {
    pub fn main_stat_value(
        &self,
        item: &EquipmentItemDefinition,
        property: &str,
        level: u32,
    ) -> Option<f32> {
        if level > item.max_level {
            return None;
        }
        let quality = match item.quality.as_str() {
            "blue" => "ITEM_QUALITY_BLUE",
            "purple" => "ITEM_QUALITY_PURPLE",
            "orange" => "ITEM_QUALITY_ORANGE",
            _ => return None,
        };
        let curve_key = match item.kind {
            EquipmentKind::Module => {
                format!("{property}_{}_{quality}", item.grid?)
            }
            EquipmentKind::Core => format!("{property}_Core_{quality}"),
        };
        interpolate_curve(self.curves.get(&curve_key)?, level as f32)
    }

    pub fn valid_module_positions(
        &self,
        character_id: u32,
        item_id: &str,
    ) -> Option<Vec<EquipmentGridCell>> {
        let plan = self.plans.get(&character_id)?;
        let item = self.items.get(item_id)?;
        if item.kind != EquipmentKind::Module {
            return None;
        }
        let shape = self.shapes.get(&item.geometry)?;
        let valid = plan.valid_cells.iter().copied().collect::<HashSet<_>>();
        let mut positions = plan
            .valid_cells
            .iter()
            .copied()
            .filter(|origin| {
                shape.cells.iter().all(|cell| {
                    valid.contains(&EquipmentGridCell {
                        row: origin.row + cell.row,
                        column: origin.column + cell.column,
                    })
                })
            })
            .collect::<Vec<_>>();
        positions.sort_by_key(|cell| (cell.row, cell.column));
        Some(positions)
    }
}

#[derive(Deserialize)]
struct CharacterDocument {
    characters: HashMap<String, CharacterInfo>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Field {
    raw: [u8; MAX_RECORD_FIELD_LENGTH],
    len: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedDamageRecord {
    pub damage: f32,
    pub target_hp_before: f32,
    pub target_max_hp: f32,
    pub damage_time: f64,
    pub world_time: f32,
    pub repeated_damage: f32,
    pub state_flags: [i32; 3],
    pub trailing_value: f32,
    pub encoding: DamageRecordEncoding,
    pub byte_offset: usize,
    pub bit_shift: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageRecordEncoding {
    LegacyInt32,
    BoolAndEnums,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedCurrentHpUpdate {
    pub current_hp: f32,
    pub byte_offset: usize,
    pub bit_shift: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedBossHpUpdate {
    /// Exact 227-bit `FCharacterForNet::NetSerialize` value, zero-padded in the
    /// high five bits of the final byte. Unlike the former 128-bit prefix this
    /// retains the per-instance discriminator used by ordinary enemies.
    pub target_handle: [u8; CLIENT_FIGHT_TARGET_WIRE_BYTES],
    pub current_hp: f32,
    pub byte_offset: usize,
    pub bit_shift: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedServerDamageSettlement {
    pub target_handle: [u8; CLIENT_FIGHT_TARGET_WIRE_BYTES],
    pub current_hp: f32,
    pub dead_state: u32,
    pub raw_damage: u32,
    pub secondary_value: i32,
    pub byte_offset: usize,
    pub bit_shift: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedGameplayEffect {
    pub unique_index: u32,
    pub byte_offset: usize,
    pub bit_shift: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParsedHtItemNetId {
    pub solt: u32,
    pub serial: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ParsedEquipmentSlot {
    pub state: i32,
    pub equipment_id: String,
    pub equip_net_id: ParsedHtItemNetId,
    pub first_step: bool,
    pub row: i32,
    pub column: i32,
    pub new_flag: i32,
    pub byte_offset: usize,
    pub bit_shift: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParsedEmptyCurtainModulePlacement {
    pub equipment: HtItemNetId,
    pub item_id: String,
    pub row: i32,
    pub column: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParsedEmptyCurtainCompactModulePlacement {
    pub equipment: HtItemNetId,
    pub item_id: String,
    pub row: i32,
    pub column: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ParsedEmptyCurtainEquipmentSnapshot {
    Modules {
        character_net_id: HtItemNetId,
        character_id: u32,
        placements: Vec<ParsedEmptyCurtainModulePlacement>,
    },
    Core {
        character_net_id: HtItemNetId,
        character_id: u32,
        item_id: Option<HtItemNetId>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameplayEffectSkill {
    pub damage_source_category: Option<String>,
    pub ability_name: Option<String>,
    pub attack_type: String,
    pub damage_component: Option<String>,
    pub owner_character_id: Option<u32>,
}

pub fn find_data_file(relative_path: &Path) -> Option<PathBuf> {
    resource_file_path(relative_path)
        .or_else(|| resource_exists(relative_path).then(|| relative_path.to_path_buf()))
}

pub fn load_characters(path: &Path) -> Result<HashMap<u32, CharacterInfo>> {
    let text =
        read_resource_text(path).with_context(|| format!("无法读取角色表 {}", path.display()))?;
    let document: CharacterDocument = serde_json::from_str(&text).context("角色表 JSON 无效")?;
    Ok(document
        .characters
        .into_iter()
        .filter_map(|(key, value)| key.parse::<u32>().ok().map(|id| (id, value)))
        .collect())
}

pub fn enemy_config_hash(config_id: &str) -> Option<u64> {
    if config_id.is_empty() || !config_id.is_ascii() {
        return None;
    }
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in config_id.bytes() {
        hash ^= u64::from(byte.to_ascii_lowercase());
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Some(hash)
}

pub fn load_enemy_catalog(path: &Path) -> Result<EnemyCatalog> {
    let text = read_resource_text(path)
        .with_context(|| format!("Failed to read enemy catalog {}", path.display()))?;
    let document: EnemyCatalogDocument =
        serde_json::from_str(&text).context("Invalid enemy catalog JSON")?;
    ensure!(document.version == 1, "Unsupported enemy catalog version");
    ensure!(
        document.hash == "fnv1a64_ascii_lower",
        "Unsupported enemy catalog hash"
    );
    ensure!(!document.enemies.is_empty(), "Enemy catalog has no entries");

    let mut enemies = HashMap::with_capacity(document.enemies.len());
    let mut config_ids = HashSet::with_capacity(document.enemies.len());
    for entry in document.enemies {
        ensure!(
            config_ids.insert(entry.config_id.clone()),
            "Duplicate enemy config ID {}",
            entry.config_id
        );
        ensure!(
            !entry.monster_id.is_empty()
                && !entry.name_en.is_empty()
                && !entry.name_zh.is_empty()
                && !entry.name_ja.is_empty(),
            "Enemy catalog entry {} has empty display data",
            entry.config_id
        );
        ensure!(
            entry.config_hash.len() == 16
                && entry
                    .config_hash
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()),
            "Enemy catalog entry {} has an invalid hash",
            entry.config_id
        );
        let config_hash =
            u64::from_str_radix(&entry.config_hash, 16).context("Invalid enemy config hash")?;
        ensure!(
            enemy_config_hash(&entry.config_id) == Some(config_hash),
            "Enemy catalog hash mismatch for {}",
            entry.config_id
        );
        let identity = EnemyIdentity {
            config_hash,
            config_id: entry.config_id,
            monster_id: entry.monster_id,
            name_en: entry.name_en,
            name_zh: entry.name_zh,
            name_ja: entry.name_ja,
        };
        ensure!(
            enemies.insert(config_hash, identity).is_none(),
            "Enemy catalog contains a config hash collision"
        );
    }
    Ok(EnemyCatalog { enemies })
}

pub fn load_equipment_catalog(path: &Path) -> Result<EquipmentCatalog> {
    let text = read_resource_text(path)
        .with_context(|| format!("Failed to read Console equipment data {}", path.display()))?;
    let catalog: EquipmentCatalog =
        serde_json::from_str(&text).context("Invalid Console equipment JSON")?;
    validate_equipment_catalog(&catalog)?;
    Ok(catalog)
}

fn validate_equipment_catalog(catalog: &EquipmentCatalog) -> Result<()> {
    ensure!(
        catalog.version == 2,
        "Unsupported Console equipment data version"
    );
    ensure!(
        !catalog.items.is_empty(),
        "Console equipment data has no items"
    );
    ensure!(
        !catalog.attributes.is_empty(),
        "Console equipment data has no attributes"
    );
    ensure!(
        !catalog.curves.is_empty(),
        "Console equipment data has no main-stat curves"
    );
    let max_level = catalog
        .items
        .values()
        .map(|item| item.max_level)
        .max()
        .unwrap_or(0) as f32;

    for (item_id, item) in &catalog.items {
        ensure!(
            !item_id.is_empty(),
            "Console equipment data has an empty item_id"
        );
        ensure!(
            !item.name_zh.is_empty() && !item.name_en.is_empty() && !item.name_ja.is_empty(),
            "Console equipment item {item_id} is missing localized names"
        );
        ensure!(
            matches!(item.quality.as_str(), "blue" | "purple" | "orange"),
            "Console equipment item {item_id} has an invalid quality"
        );
        ensure!(
            !item.geometry.is_empty(),
            "Console equipment item {item_id} is missing geometry"
        );
        ensure!(
            item.icon.starts_with("res/images/") && item.icon.ends_with(".png"),
            "Console equipment item {item_id} has an invalid icon path"
        );
        ensure!(
            item.max_level <= 100,
            "Console equipment item {item_id} has an invalid level cap"
        );
        ensure!(
            (1..=8).contains(&item.main_count),
            "Console equipment item {item_id} has an invalid main-stat count"
        );
        ensure!(
            item.sub_count <= 16,
            "Console equipment item {item_id} has an invalid substat count"
        );
        ensure!(
            item.main_count + item.sub_count <= EMPTY_CURTAIN_MAX_STAT_ROWS,
            "Console equipment item {item_id} has too many stat rows"
        );
        match item.kind {
            EquipmentKind::Module => {
                ensure!(
                    item.grid.is_some_and(|grid| grid > 0),
                    "Console module {item_id} is missing its grid size"
                );
                ensure!(
                    catalog.shapes.contains_key(&item.geometry),
                    "Console module {item_id} references unknown shape {}",
                    item.geometry
                );
            }
            EquipmentKind::Core => {
                ensure!(
                    item.grid.is_none(),
                    "Console cassette {item_id} has a grid size"
                );
                ensure!(
                    item.suit.is_some(),
                    "Console cassette {item_id} is missing a suit"
                );
            }
        }
        if let Some(suit) = &item.suit {
            ensure!(
                catalog.suits.contains_key(suit),
                "Console equipment item {item_id} references unknown suit {suit}"
            );
        }
    }

    ensure!(
        !catalog.shapes.is_empty(),
        "Console equipment data has no module shapes"
    );
    for (shape_id, shape) in &catalog.shapes {
        let unique = shape.cells.iter().copied().collect::<HashSet<_>>();
        ensure!(
            !shape_id.is_empty()
                && !shape.cells.is_empty()
                && unique.len() == shape.cells.len()
                && unique.contains(&EquipmentGridCell { row: 0, column: 0 }),
            "Console equipment shape {shape_id} is invalid"
        );
    }

    ensure!(
        !catalog.plans.is_empty(),
        "Console equipment data has no character plans"
    );
    for (character_id, plan) in &catalog.plans {
        ensure!(
            *character_id != 0,
            "Console equipment plan has character ID 0"
        );
        let valid = plan.valid_cells.iter().copied().collect::<HashSet<_>>();
        ensure!(
            valid.len() == plan.valid_cells.len()
                && plan
                    .valid_cells
                    .iter()
                    .all(|cell| (1..=5).contains(&cell.row) && (1..=5).contains(&cell.column)),
            "Console equipment plan {character_id} has invalid template cells"
        );
        ensure!(
            catalog
                .items
                .get(&plan.recommended_core)
                .is_some_and(|item| item.kind == EquipmentKind::Core),
            "Console equipment plan {character_id} has an invalid recommended core"
        );

        let mut occupied = HashSet::new();
        for module in &plan.recommended_modules {
            let definition = catalog.items.get(&module.item_id).with_context(|| {
                format!(
                    "Console equipment plan {character_id} references unknown module {}",
                    module.item_id
                )
            })?;
            ensure!(
                definition.kind == EquipmentKind::Module,
                "Console equipment plan {character_id} references a non-module item"
            );
            let shape = catalog.shapes.get(&definition.geometry).with_context(|| {
                format!(
                    "Console equipment plan {character_id} references unknown shape {}",
                    definition.geometry
                )
            })?;
            for cell in &shape.cells {
                let occupied_cell = EquipmentGridCell {
                    row: module.row + cell.row,
                    column: module.column + cell.column,
                };
                ensure!(
                    valid.contains(&occupied_cell),
                    "Console equipment plan {character_id} leaves its character template"
                );
                ensure!(
                    occupied.insert(occupied_cell),
                    "Console equipment plan {character_id} has overlapping modules"
                );
            }
        }
    }

    for (property, attribute) in &catalog.attributes {
        ensure!(
            !property.is_empty(),
            "Console equipment data has an empty property ID"
        );
        ensure!(
            !attribute.name_zh.is_empty()
                && !attribute.name_en.is_empty()
                && !attribute.name_ja.is_empty(),
            "Console equipment property {property} is missing localized names"
        );
    }

    for (curve_id, points) in &catalog.curves {
        ensure!(
            !points.is_empty(),
            "Console equipment curve {curve_id} has no points"
        );
        let mut previous_time = None;
        for [time, value] in points {
            ensure!(
                time.is_finite() && value.is_finite(),
                "Console equipment curve {curve_id} contains a non-finite value"
            );
            ensure!(
                *time >= 0.0,
                "Console equipment curve {curve_id} contains a negative level"
            );
            if let Some(previous) = previous_time {
                ensure!(
                    *time > previous,
                    "Console equipment curve {curve_id} levels are not strictly increasing"
                );
            }
            previous_time = Some(*time);
        }
        ensure!(
            points.first().is_some_and(|point| point[0] == 0.0)
                && points.last().is_some_and(|point| point[0] >= max_level),
            "Console equipment curve {curve_id} does not cover the supported level range"
        );
    }

    for (suit_id, suit) in &catalog.suits {
        let suffix_len = suit_id
            .as_bytes()
            .iter()
            .rev()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        ensure!(
            suffix_len > 0 && suit_id[suit_id.len() - suffix_len..].parse::<u32>().is_ok(),
            "Console equipment suit {suit_id} has an invalid numeric suffix"
        );
        ensure!(
            !suit.name_zh.is_empty() && !suit.name_en.is_empty() && !suit.name_ja.is_empty(),
            "Console equipment suit {suit_id} is missing localized names"
        );
        ensure!(
            !suit.effects.is_empty(),
            "Console equipment suit {suit_id} has no effects"
        );
        for effect in &suit.effects {
            ensure!(
                effect.count > 0,
                "Console equipment suit {suit_id} has an invalid piece count"
            );
            ensure!(
                !effect.text_zh.is_empty()
                    && !effect.text_en.is_empty()
                    && !effect.text_ja.is_empty(),
                "Console equipment suit {suit_id} is missing effect text"
            );
        }
    }
    Ok(())
}

fn interpolate_curve(points: &[[f32; 2]], time: f32) -> Option<f32> {
    let first = *points.first()?;
    if time <= first[0] {
        return Some(first[1]);
    }
    for pair in points.windows(2) {
        let [left, right] = pair else {
            continue;
        };
        if time <= right[0] {
            let progress = (time - left[0]) / (right[0] - left[0]);
            return Some(left[1] + (right[1] - left[1]) * progress);
        }
    }
    points.last().map(|point| point[1])
}

pub fn load_gameplay_effect_mapping(path: &Path) -> Result<HashMap<u32, String>> {
    let text = read_resource_text(path)
        .with_context(|| format!("无法读取 GameplayEffect 映射表 {}", path.display()))?;
    let document: serde_json::Value =
        serde_json::from_str(&text).context("GameplayEffect 映射表 JSON 无效")?;
    if let Some(effects) = document
        .get("effects")
        .and_then(serde_json::Value::as_object)
    {
        return Ok(effects
            .iter()
            .filter_map(|(index, name)| {
                index.parse::<u32>().ok().and_then(|index| {
                    name.as_str()
                        .filter(|name| index != 0 && !name.is_empty())
                        .map(|name| (index, name.to_owned()))
                })
            })
            .collect());
    }
    let rows = document
        .as_array()
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.get("Rows"))
        .and_then(serde_json::Value::as_object)
        .context("GameplayEffect 映射表缺少 Rows")?;

    Ok(rows
        .iter()
        .filter_map(|(name, row)| {
            row.get("UniqueIndex")
                .and_then(serde_json::Value::as_u64)
                .and_then(|index| u32::try_from(index).ok())
                .filter(|index| *index != 0)
                .map(|index| (index, name.clone()))
        })
        .collect())
}

#[derive(Clone, Debug, Default)]
pub struct AbilityCatalog {
    skills: HashMap<String, GameplayEffectSkill>,
}

impl AbilityCatalog {
    pub fn load(path: &Path) -> Result<Self> {
        load_gameplay_effect_skills(path).map(Self::from)
    }

    pub fn apply_semantics(&mut self, path: &Path) -> Result<()> {
        let semantics = load_gameplay_effect_semantics(path)?;
        for effect_name in semantics.keys() {
            ensure!(
                self.skills.contains_key(effect_name),
                "GE 语义表引用了技能表中不存在的 {effect_name}"
            );
        }
        for (effect_name, semantic) in semantics {
            let skill = self
                .skills
                .get_mut(&effect_name)
                .expect("GE semantics keys were validated against the skill catalog");
            if let Some(ability_name) = semantic.ability {
                skill.ability_name = Some(ability_name);
            }
            if let Some(attack_type) = semantic.attack_type {
                skill.attack_type = attack_type;
            }
            if let Some(damage_name_en) = semantic.damage_name_en {
                skill.damage_component = Some(damage_name_en);
            }
            skill.owner_character_id = semantic.owner_character_id;
        }
        Ok(())
    }

    pub fn skill(&self, effect_name: &str) -> Option<&GameplayEffectSkill> {
        self.skills.get(effect_name)
    }

    pub fn ability_name(&self, effect_name: &str) -> Option<&str> {
        self.skill(effect_name)?.ability_name.as_deref()
    }
}

#[derive(Clone, Debug, Deserialize)]
struct GameplayEffectSemanticDocument {
    format_version: u32,
    effects: HashMap<String, GameplayEffectSemantic>,
}

#[derive(Clone, Debug, Deserialize)]
struct GameplayEffectSemantic {
    #[serde(default)]
    owner_character_id: Option<u32>,
    #[serde(default)]
    ability: Option<String>,
    #[serde(default)]
    attack_type: Option<String>,
    #[serde(default = "default_show_parent_ability")]
    show_parent_ability: bool,
    #[serde(default)]
    damage_name_en: Option<String>,
    #[serde(default)]
    damage_name_zh: Option<String>,
    #[serde(default)]
    damage_name_ja: Option<String>,
}

const fn default_show_parent_ability() -> bool {
    true
}

fn load_gameplay_effect_semantics(path: &Path) -> Result<HashMap<String, GameplayEffectSemantic>> {
    let text = read_resource_text(path)
        .with_context(|| format!("无法读取 GE 语义表 {}", path.display()))?;
    let document: GameplayEffectSemanticDocument =
        serde_json::from_str(&text).context("GE 语义表 JSON 无效")?;
    ensure!(
        document.format_version == 1,
        "GE 语义表 format_version {} 暂未支持",
        document.format_version
    );
    for (effect_name, semantic) in &document.effects {
        ensure!(
            effect_name.starts_with("GE_") || effect_name.starts_with("Buff_"),
            "GE 语义表包含无效标识 {effect_name}"
        );
        ensure!(
            semantic.owner_character_id != Some(0),
            "GE 语义表 {effect_name} 的 owner_character_id 无效"
        );
        ensure!(
            semantic
                .ability
                .as_deref()
                .is_none_or(|ability| !ability.trim().is_empty()),
            "GE 语义表 {effect_name} 的 ability 为空"
        );
        ensure!(
            semantic
                .ability
                .as_deref()
                .is_none_or(|ability| ability.starts_with("GA_")),
            "GE 语义表 {effect_name} 的 ability 标识无效"
        );
        ensure!(
            semantic
                .attack_type
                .as_deref()
                .is_none_or(|attack_type| !attack_type.trim().is_empty()),
            "GE 语义表 {effect_name} 的 attack_type 为空"
        );
        let damage_names = [
            semantic.damage_name_en.as_deref(),
            semantic.damage_name_zh.as_deref(),
            semantic.damage_name_ja.as_deref(),
        ];
        ensure!(
            damage_names.iter().all(Option::is_none)
                || damage_names
                    .iter()
                    .all(|name| name.is_some_and(|name| !name.trim().is_empty())),
            "GE 语义表 {effect_name} 的多语言伤害组件名必须同时提供"
        );
    }
    Ok(document.effects)
}

pub fn load_gameplay_effect_semantic_names(
    path: &Path,
    language: Language,
) -> Result<HashMap<String, (String, bool)>> {
    let semantics = load_gameplay_effect_semantics(path)?;
    Ok(semantics
        .into_iter()
        .filter_map(|(effect_name, semantic)| {
            let name = match language {
                Language::SimplifiedChinese => semantic.damage_name_zh,
                Language::English => semantic.damage_name_en,
                Language::Japanese => semantic.damage_name_ja,
            };
            name.map(|name| (effect_name, (name, semantic.show_parent_ability)))
        })
        .collect())
}

impl From<HashMap<String, GameplayEffectSkill>> for AbilityCatalog {
    fn from(skills: HashMap<String, GameplayEffectSkill>) -> Self {
        Self { skills }
    }
}

pub fn load_gameplay_effect_skills(path: &Path) -> Result<HashMap<String, GameplayEffectSkill>> {
    let text = read_resource_text(path)
        .with_context(|| format!("无法读取技能伤害表 {}", path.display()))?;
    let document: serde_json::Value =
        serde_json::from_str(&text).context("技能伤害表 JSON 无效")?;
    if let Some(skills) = document
        .get("skills")
        .and_then(serde_json::Value::as_object)
    {
        return Ok(skills
            .iter()
            .map(|(effect_name, row)| {
                let category = row
                    .get("category")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned);
                let ability_name = row
                    .get("ability")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.is_empty() && *value != "None")
                    .map(str::to_owned);
                let attack_type =
                    classify_attack_type(category.as_deref(), effect_name, ability_name.as_deref());
                (
                    effect_name.clone(),
                    GameplayEffectSkill {
                        damage_source_category: category,
                        ability_name,
                        attack_type,
                        damage_component: None,
                        owner_character_id: None,
                    },
                )
            })
            .collect());
    }
    let rows = document
        .as_array()
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.get("Rows"))
        .and_then(serde_json::Value::as_object)
        .context("技能伤害表缺少 Rows")?;

    Ok(rows
        .iter()
        .map(|(effect_name, row)| {
            let category = row
                .get("DamageSourceCategory")
                .and_then(serde_json::Value::as_str)
                .and_then(damage_source_category_code)
                .map(str::to_owned);
            let ability_name = row
                .get("GAName")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty() && *value != "None")
                .map(str::to_owned);
            let attack_type =
                classify_attack_type(category.as_deref(), effect_name, ability_name.as_deref());
            (
                effect_name.clone(),
                GameplayEffectSkill {
                    damage_source_category: category,
                    ability_name,
                    attack_type,
                    damage_component: None,
                    owner_character_id: None,
                },
            )
        })
        .collect())
}

/// Maps GA_ ability names to their official in-game skill name, sourced from
/// `DT_GameplayAbilityTipsData`. This table is keyed by ability rather than by
/// GameplayEffect, so callers join it through [`GameplayEffectSkill::ability_name`].
///
/// Picks the field matching `language`, falling back through the other fields
/// when the active one is empty (the Global asset export doesn't localize
/// every ability into every language, e.g. brand-new or CN-exclusive skills)
/// so a skill name is shown whenever any language has one, rather than
/// disappearing. Takes `language` explicitly (rather than reading
/// `i18n::current_language()` internally) so it stays a pure function of its
/// arguments — callers pass the live language, tests pass a fixed one.
pub fn load_ability_tip_names(path: &Path, language: Language) -> Result<HashMap<String, String>> {
    let text = read_resource_text(path)
        .with_context(|| format!("无法读取技能说明表 {}", path.display()))?;
    let document: serde_json::Value =
        serde_json::from_str(&text).context("技能说明表 JSON 无效")?;
    let abilities = document
        .get("abilities")
        .and_then(serde_json::Value::as_object)
        .context("技能说明表缺少 abilities")?;
    let field_priority = ability_name_field_priority(language);
    let mut names = abilities
        .iter()
        .filter_map(|(ability_name, row)| {
            // Some name fields ship the game's rich-text markup baked in (e.g.
            // `<Title>变轨技能：轮转打击</>`); strip it to the plain text the
            // player sees, and fall through to the next language when a field is
            // empty or reduces to nothing.
            let name = field_priority.iter().find_map(|field| {
                let raw = row.get(*field).and_then(serde_json::Value::as_str)?;
                let name = crate::engine::rich_text::strip_tags(raw);
                (!name.is_empty()).then_some(name)
            })?;
            Some((ability_name.clone(), name))
        })
        .collect::<HashMap<_, _>>();
    for (alias, source) in ABILITY_DISPLAY_NAME_ALIASES {
        if !names.contains_key(alias)
            && let Some(name) = names.get(source).cloned()
        {
            names.insert(alias.to_owned(), name);
        }
    }
    Ok(names)
}

fn ability_name_field_priority(language: Language) -> [&'static str; 3] {
    match language {
        Language::Japanese => ["name_ja", "name_zh", "name_en"],
        Language::English => ["name_en", "name_zh", "name_ja"],
        Language::SimplifiedChinese => ["name_zh", "name_en", "name_ja"],
    }
}

pub fn normalize_damage_name(description: &str) -> String {
    description
        .replace("QTE", "环合")
        .chars()
        .filter(|character| !character.is_ascii_digit() && *character != '_')
        .collect::<String>()
        .trim()
        .to_owned()
}

fn damage_source_category_code(value: &str) -> Option<&str> {
    value
        .strip_prefix("EExecutionDamageSourceCategory::DAMAGE_SOURCE_CATEGORY_")
        .or_else(|| value.rsplit('_').next())
        .filter(|value| !value.is_empty() && *value != "NULL")
}

pub fn classify_attack_type(
    category: Option<&str>,
    effect_name: &str,
    ability_name: Option<&str>,
) -> String {
    let searchable = format!("{} {}", effect_name, ability_name.unwrap_or_default());
    let searchable_lower = searchable.to_ascii_lowercase();
    if searchable_lower.contains("tenacity") {
        return "倾陷伤害".to_owned();
    }
    if searchable_lower.contains("parry") {
        return "格挡反击".to_owned();
    }
    if ability_name.is_some_and(|name| name.starts_with("GA_CardTrigger_"))
        || (effect_name.starts_with("GE_AbyssCard_") && effect_name.contains("_Damage"))
    {
        return "深渊场地Buff".to_owned();
    }
    if effect_name.contains("Reaction_1") || effect_name.contains("Reaction1_") {
        return "创生花".to_owned();
    }
    if effect_name.contains("Reaction_2") || effect_name.contains("Reaction2_") {
        return "覆纹".to_owned();
    }
    if effect_name.contains("Reaction_3") || effect_name.contains("Reaction3_") {
        return "延滞".to_owned();
    }
    if effect_name.contains("Reaction_4") || effect_name.contains("Reaction4_") {
        return "黯星".to_owned();
    }
    if effect_name.contains("Reaction_5") || effect_name.contains("Reaction5_") {
        return "浊燃".to_owned();
    }
    if effect_name.contains("Reaction_6") || effect_name.contains("Reaction6_") {
        return "浸染".to_owned();
    }
    if effect_name.contains("Reaction_7") || effect_name.contains("Reaction7_") {
        return "盈蓄".to_owned();
    }
    if effect_name.contains("Reaction_8")
        || effect_name.contains("Reaction8_")
        || effect_name.contains("AnHunZhou")
    {
        return "失谐".to_owned();
    }

    let category_type = match category {
        Some("A") => Some("普攻"),
        Some("E") => Some("E技能"),
        Some("Q") => Some("Q技能"),
        Some("H") => Some("环合"),
        Some("R") => Some("环合伤害"),
        Some("Z") => Some("闪避反击"),
        _ => None,
    };
    if let Some(attack_type) = category_type {
        return attack_type.to_owned();
    }

    if searchable.contains("UltraSkill") {
        "Q技能".to_owned()
    } else if searchable.contains("QTE") || searchable.contains("EntryAttack") {
        "环合".to_owned()
    } else if searchable.contains("Melee") || searchable.contains("NormalAttack") {
        "普攻".to_owned()
    } else if searchable.contains("Skill") {
        "E技能".to_owned()
    } else {
        "其他".to_owned()
    }
}

pub fn qte_reaction_type(
    previous_attribute: &str,
    entering_attribute: &str,
) -> Option<&'static str> {
    let has_pair = |left: &str, right: &str| {
        (previous_attribute == left && entering_attribute == right)
            || (previous_attribute == right && entering_attribute == left)
    };

    if has_pair("光", "灵") {
        Some("创生")
    } else if has_pair("灵", "咒") {
        Some("覆纹")
    } else if has_pair("光", "相") {
        Some("延滞")
    } else if has_pair("暗", "魂") {
        Some("黯星")
    } else if has_pair("暗", "咒") {
        Some("浊燃")
    } else if has_pair("魂", "相") {
        Some("浸染")
    } else {
        None
    }
}

fn decode_shifted_into(
    data: &[u8],
    byte_offset: usize,
    bit_shift: u8,
    start_bit_offset: usize,
    output: &mut [u8],
) -> Option<()> {
    for (index, byte) in output.iter_mut().enumerate() {
        let bit_position = bit_shift as usize + start_bit_offset + index * 8;
        let source_offset = byte_offset + bit_position / 8;
        let source_shift = bit_position % 8;
        let current = *data.get(source_offset)?;
        let mut value = (current as u16) >> source_shift;
        if source_shift != 0 {
            value |= (*data.get(source_offset + 1)? as u16) << (8 - source_shift);
        }
        *byte = value as u8;
    }
    Some(())
}

fn decode_shifted_bytes(
    data: &[u8],
    byte_offset: usize,
    bit_shift: u8,
    start_bit_offset: usize,
    count: usize,
) -> Option<Vec<u8>> {
    let mut output = vec![0; count];
    decode_shifted_into(data, byte_offset, bit_shift, start_bit_offset, &mut output)?;
    Some(output)
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn aligned_bytes_for_test(data: &[u8], bit_shift: u8) -> Option<Vec<u8>> {
    decode_shifted_bytes(data, 0, bit_shift, 0, data.len().saturating_sub(1))
}

fn read_field(
    data: &[u8],
    byte_offset: usize,
    bit_shift: u8,
    bit_offset: usize,
) -> Option<(u8, Field, usize)> {
    let mut header = [0; 5];
    decode_shifted_into(data, byte_offset, bit_shift, bit_offset, &mut header)?;
    let field_length = u32::from_le_bytes(header[1..5].try_into().ok()?) as usize;
    let consumed_bits = 40 + field_length * 8;
    let remaining_bits = data.len().saturating_sub(byte_offset) * 8;
    if field_length == 0
        || field_length > MAX_RECORD_FIELD_LENGTH
        || bit_offset + consumed_bits > remaining_bits
    {
        return None;
    }
    let mut field = Field {
        len: field_length,
        ..Default::default()
    };
    decode_shifted_into(
        data,
        byte_offset,
        bit_shift,
        bit_offset + 40,
        &mut field.raw[..field_length],
    )?;
    Some((header[0], field, consumed_bits))
}

fn f32_field(field: &Field) -> Option<f32> {
    Some(f32::from_le_bytes(field.raw[..field.len].try_into().ok()?))
}

fn f64_field(field: &Field) -> Option<f64> {
    Some(f64::from_le_bytes(field.raw[..field.len].try_into().ok()?))
}

fn i32_field(field: &Field) -> Option<i32> {
    Some(i32::from_le_bytes(field.raw[..field.len].try_into().ok()?))
}

fn read_i32_le(data: &[u8], offset: usize) -> Option<(i32, usize)> {
    Some((
        i32::from_le_bytes(data.get(offset..offset + 4)?.try_into().ok()?),
        offset + 4,
    ))
}

fn read_u32_le(data: &[u8], offset: usize) -> Option<(u32, usize)> {
    Some((
        u32::from_le_bytes(data.get(offset..offset + 4)?.try_into().ok()?),
        offset + 4,
    ))
}

fn read_u8(data: &[u8], offset: usize) -> Option<(u8, usize)> {
    Some((*data.get(offset)?, offset + 1))
}

fn read_fstring(data: &[u8], offset: usize) -> Option<(&str, usize)> {
    let (length, cursor) = read_i32_le(data, offset)?;
    if !(1..=MAX_TAGGED_PROPERTY_STRING_LENGTH).contains(&length) {
        return None;
    }
    let length = usize::try_from(length).ok()?;
    let raw = data.get(cursor..cursor + length)?;
    if raw.last() != Some(&0) {
        return None;
    }
    let value = std::str::from_utf8(&raw[..length - 1]).ok()?;
    if value.as_bytes().contains(&0) {
        return None;
    }
    Some((value, cursor + length))
}

fn read_property_header(
    data: &[u8],
    offset: usize,
    expected_name: &str,
    expected_type: &str,
) -> Option<(usize, i32, i32)> {
    let (name, cursor) = read_fstring(data, offset)?;
    if name != expected_name {
        return None;
    }
    let (property_type, cursor) = read_fstring(data, cursor)?;
    if property_type != expected_type {
        return None;
    }
    let (array_index, cursor) = read_i32_le(data, cursor)?;
    let (size, cursor) = read_i32_le(data, cursor)?;
    Some((cursor, array_index, size))
}

fn skip_property_guid(data: &[u8], offset: usize) -> Option<usize> {
    let (has_guid, cursor) = read_u8(data, offset)?;
    if has_guid == 0 {
        Some(cursor)
    } else {
        data.get(cursor..cursor + 16)?;
        Some(cursor + 16)
    }
}

fn parse_i32_property(data: &[u8], offset: usize, expected_name: &str) -> Option<(i32, usize)> {
    let (cursor, array_index, size) =
        read_property_header(data, offset, expected_name, "IntProperty")?;
    if array_index != 0 || size != 4 {
        return None;
    }
    let cursor = skip_property_guid(data, cursor)?;
    read_i32_le(data, cursor)
}

fn parse_u32_property(data: &[u8], offset: usize, expected_name: &str) -> Option<(u32, usize)> {
    let (cursor, array_index, size) =
        read_property_header(data, offset, expected_name, "UInt32Property")?;
    if array_index != 0 || size != 4 {
        return None;
    }
    let cursor = skip_property_guid(data, cursor)?;
    read_u32_le(data, cursor)
}

fn parse_name_property(data: &[u8], offset: usize, expected_name: &str) -> Option<(String, usize)> {
    let (cursor, array_index, size) =
        read_property_header(data, offset, expected_name, "NameProperty")?;
    if array_index != 0 || size <= 0 {
        return None;
    }
    let cursor = skip_property_guid(data, cursor)?;
    let (value, cursor) = read_fstring(data, cursor)?;
    Some((value.to_owned(), cursor))
}

fn parse_bool_property(data: &[u8], offset: usize, expected_name: &str) -> Option<(bool, usize)> {
    let (cursor, array_index, size) =
        read_property_header(data, offset, expected_name, "BoolProperty")?;
    if array_index != 0 || size != 0 {
        return None;
    }
    let (value, cursor) = read_u8(data, cursor)?;
    Some((value != 0, cursor))
}

fn parse_none_property(data: &[u8], offset: usize) -> Option<usize> {
    let (name, cursor) = read_fstring(data, offset)?;
    (name == "None").then_some(cursor)
}

fn parse_ht_item_net_id_property(
    data: &[u8],
    offset: usize,
    expected_name: &str,
) -> Option<(ParsedHtItemNetId, usize)> {
    let (name, cursor) = read_fstring(data, offset)?;
    if name != expected_name {
        return None;
    }
    let (property_type, cursor) = read_fstring(data, cursor)?;
    if property_type != "StructProperty" {
        return None;
    }
    let (_array_index, cursor) = read_i32_le(data, cursor)?;
    let (struct_name, cursor) = read_fstring(data, cursor)?;
    if struct_name != "HTItemNetID" {
        return None;
    }
    let (_struct_index, cursor) = read_i32_le(data, cursor)?;
    let (struct_path, cursor) = read_fstring(data, cursor)?;
    if struct_path != "/Script/HTGame" {
        return None;
    }
    let (_reserved, cursor) = read_i32_le(data, cursor)?;
    let (_nested_size, cursor) = read_i32_le(data, cursor)?;
    let cursor = skip_property_guid(data, cursor)?;
    let (solt, cursor) = parse_u32_property(data, cursor, "solt")?;
    let (serial, cursor) = parse_u32_property(data, cursor, "serial")?;
    let cursor = parse_none_property(data, cursor)?;
    Some((ParsedHtItemNetId { solt, serial }, cursor))
}

fn parse_equipment_slot_at(
    data: &[u8],
    offset: usize,
    bit_shift: u8,
) -> Option<(ParsedEquipmentSlot, usize)> {
    let (state, cursor) = parse_i32_property(data, offset, "State")?;
    let (equipment_id, cursor) = parse_name_property(data, cursor, "EquipmentID")?;
    let (equip_net_id, cursor) = parse_ht_item_net_id_property(data, cursor, "EquipNetID")?;
    let (first_step, cursor) = parse_bool_property(data, cursor, "bFirstStep")?;
    let (row, cursor) = parse_i32_property(data, cursor, "Row")?;
    let (column, cursor) = parse_i32_property(data, cursor, "Column")?;
    let (new_flag, cursor) = parse_i32_property(data, cursor, "New")?;
    let cursor = parse_none_property(data, cursor)?;
    Some((
        ParsedEquipmentSlot {
            state,
            equipment_id,
            equip_net_id,
            first_step,
            row,
            column,
            new_flag,
            byte_offset: offset,
            bit_shift,
        },
        cursor,
    ))
}

fn find_bytes(data: &[u8], needle: &[u8]) -> Option<usize> {
    data.windows(needle.len())
        .position(|window| window == needle)
}

pub fn parse_equipment_slots(data: &[u8]) -> Vec<ParsedEquipmentSlot> {
    let mut slots = Vec::new();
    for bit_shift in 0..8_u8 {
        let shifted_storage;
        let shifted = if bit_shift == 0 {
            data
        } else {
            shifted_storage =
                match decode_shifted_bytes(data, 0, bit_shift, 0, data.len().saturating_sub(1)) {
                    Some(value) => value,
                    None => continue,
                };
            shifted_storage.as_slice()
        };
        let mut search_offset = 0;
        while let Some(relative_offset) =
            find_bytes(&shifted[search_offset..], EQUIPMENT_SLOT_STATE_ANCHOR)
        {
            let slot_offset = search_offset + relative_offset;
            match parse_equipment_slot_at(shifted, slot_offset, bit_shift) {
                Some((slot, next_offset)) => {
                    slots.push(slot);
                    search_offset = next_offset.max(slot_offset + 1);
                }
                None => {
                    search_offset = slot_offset + 1;
                }
            }
        }
    }
    slots
}

#[derive(Clone, Copy)]
struct InventoryBitReader<'a> {
    data: &'a [u8],
    bit_len: usize,
    cursor: usize,
}

impl<'a> InventoryBitReader<'a> {
    fn at(data: &'a [u8], bit_len: usize, cursor: usize) -> Option<Self> {
        if bit_len > data.len().checked_mul(8)? || cursor > bit_len {
            return None;
        }
        Some(Self {
            data,
            bit_len,
            cursor,
        })
    }

    fn read_bits(&mut self, count: usize) -> Option<u64> {
        if count > 64 || self.cursor.checked_add(count)? > self.bit_len {
            return None;
        }
        let mut value = 0_u64;
        for index in 0..count {
            let source = self.cursor + index;
            value |= u64::from((self.data[source / 8] >> (source % 8)) & 1) << index;
        }
        self.cursor += count;
        Some(value)
    }

    fn read_bool(&mut self) -> Option<bool> {
        Some(self.read_bits(1)? != 0)
    }

    fn read_u8(&mut self) -> Option<u8> {
        Some(self.read_bits(8)? as u8)
    }

    fn read_u16(&mut self) -> Option<u16> {
        Some(self.read_bits(16)? as u16)
    }

    fn read_u32(&mut self) -> Option<u32> {
        Some(self.read_bits(32)? as u32)
    }

    fn read_i32(&mut self) -> Option<i32> {
        Some(self.read_u32()? as i32)
    }

    fn read_i64(&mut self) -> Option<i64> {
        Some(self.read_bits(64)? as i64)
    }

    fn read_f32(&mut self) -> Option<f32> {
        Some(f32::from_bits(self.read_u32()?))
    }

    fn read_bytes(&mut self, length: usize) -> Option<Vec<u8>> {
        let bit_count = length.checked_mul(8)?;
        self.cursor.checked_add(bit_count)?;
        if self.cursor + bit_count > self.bit_len {
            return None;
        }
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..length {
            bytes.push(self.read_u8()?);
        }
        Some(bytes)
    }

    fn read_fstring(&mut self, max_units: usize) -> Option<String> {
        let length = self.read_i32()?;
        if length == 0 {
            return Some(String::new());
        }
        if length > 0 {
            let length = usize::try_from(length).ok()?;
            if length > max_units {
                return None;
            }
            let bytes = self.read_bytes(length)?;
            if bytes.last() != Some(&0) || bytes[..length - 1].contains(&0) {
                return None;
            }
            return String::from_utf8(bytes[..length - 1].to_vec()).ok();
        }

        let unit_count = usize::try_from(length.checked_neg()?).ok()?;
        if unit_count > max_units {
            return None;
        }
        let bytes = self.read_bytes(unit_count.checked_mul(2)?)?;
        if !bytes.ends_with(&[0, 0])
            || bytes[..bytes.len() - 2]
                .as_chunks::<2>()
                .0
                .contains(&[0, 0])
        {
            return None;
        }
        String::from_utf16le(&bytes[..bytes.len() - 2]).ok()
    }

    fn read_dynamic_name(&mut self) -> Option<String> {
        if self.read_bool()? {
            return None;
        }
        let value = self.read_fstring(MAX_INVENTORY_NAME_LENGTH)?;
        if value.is_empty() || self.read_u32()? != 0 {
            return None;
        }
        Some(value)
    }

    fn read_item_net_id(&mut self) -> Option<HtItemNetId> {
        Some(HtItemNetId {
            solt: self.read_u32()?,
            serial: self.read_u32()?,
        })
    }
}

pub(crate) fn valid_item_net_id(id: HtItemNetId) -> bool {
    id.solt != 0 && id.serial != 0 && id.solt != u32::MAX && id.serial != u32::MAX
}

fn parse_empty_curtain_character_owner_at(
    mut reader: InventoryBitReader<'_>,
) -> Option<(HtItemNetId, u32)> {
    let character_id = reader.read_dynamic_name()?.parse::<u32>().ok()?;
    if character_id == 0 {
        return None;
    }
    let net_id = reader.read_item_net_id()?;
    if !valid_item_net_id(net_id) || reader.read_i64()? != 1 {
        return None;
    }

    let _durable = reader.read_i32()?;
    if reader.read_i64()? < 0 || reader.read_u16()? != 1 {
        return None;
    }

    let character_level = reader.read_i32()?;
    let breakthrough_level = reader.read_i32()?;
    let fork_net_id = reader.read_item_net_id()?;
    let awaken_level = reader.read_i32()?;
    if !(1..=MAX_INVENTORY_CHARACTER_LEVEL).contains(&character_level)
        || !(0..=MAX_INVENTORY_CHARACTER_LEVEL).contains(&breakthrough_level)
        || (!fork_net_id.is_zero() && !valid_item_net_id(fork_net_id))
        || !(0..=MAX_INVENTORY_CHARACTER_LEVEL).contains(&awaken_level)
    {
        return None;
    }

    for _ in 0..7 {
        let value = reader.read_f32()?;
        if !value.is_finite() || value < 0.0 {
            return None;
        }
    }
    let _save_is_dead = reader.read_bool()?;
    if reader.read_i32()? < 0 || reader.read_u16()? > MAX_INVENTORY_CHARACTER_ARRAY_LENGTH {
        return None;
    }

    Some((net_id, character_id))
}

pub fn parse_empty_curtain_character_owners(
    data: &[u8],
    data_bit_len: usize,
) -> HashMap<HtItemNetId, u32> {
    if data_bit_len > data.len().saturating_mul(8) {
        return HashMap::new();
    }
    let mut characters = HashMap::new();
    for bit_offset in 0..data_bit_len {
        let Some(reader) = InventoryBitReader::at(data, data_bit_len, bit_offset) else {
            break;
        };
        if let Some((net_id, character_id)) = parse_empty_curtain_character_owner_at(reader) {
            characters.insert(net_id, character_id);
        }
    }
    characters
}

pub(crate) fn parse_empty_curtain_compact_module_placements(
    data: &[u8],
    data_bit_len: usize,
    known_items: &HashMap<HtItemNetId, EmptyCurtainItem>,
    catalog: &EquipmentCatalog,
) -> Vec<ParsedEmptyCurtainCompactModulePlacement> {
    if data_bit_len > data.len().saturating_mul(8) {
        return Vec::new();
    }

    let mut modules = HashMap::<
        HtItemNetId,
        (
            String,
            HashSet<EquipmentGridCell>,
            Option<EquipmentGridCell>,
        ),
    >::new();
    let mut invalid = HashSet::new();
    let mut bit_offset = 0;
    while bit_offset < data_bit_len {
        let Some(mut reader) = InventoryBitReader::at(data, data_bit_len, bit_offset) else {
            break;
        };
        let parsed = (|| {
            if reader.read_i32()? != 1 {
                return None;
            }
            let item_id = reader.read_dynamic_name()?;
            let definition = catalog.items.get(&item_id)?;
            if definition.kind != EquipmentKind::Module {
                return None;
            }
            let equipment = reader.read_item_net_id()?;
            let first_step = reader.read_bool()?;
            let row = reader.read_i32()?;
            let column = reader.read_i32()?;
            let new_flag = reader.read_i32()?;
            if !valid_item_net_id(equipment)
                || !(0..EMPTY_CURTAIN_GRID_SIDE).contains(&row)
                || !(0..EMPTY_CURTAIN_GRID_SIDE).contains(&column)
                || new_flag != 0
                || known_items
                    .get(&equipment)
                    .is_some_and(|item| item.item_id != item_id)
            {
                return None;
            }
            Some((equipment, item_id, first_step, row, column, reader.cursor))
        })();
        let Some((equipment, item_id, first_step, row, column, next_offset)) = parsed else {
            bit_offset += 1;
            continue;
        };
        let cell = EquipmentGridCell { row, column };
        let entry = modules
            .entry(equipment)
            .or_insert_with(|| (item_id.clone(), HashSet::new(), None));
        if entry.0 != item_id || !entry.1.insert(cell) {
            invalid.insert(equipment);
        }
        if first_step && entry.2.replace(cell).is_some() {
            invalid.insert(equipment);
        }
        bit_offset = next_offset.max(bit_offset + 1);
    }

    let mut placements = modules
        .into_iter()
        .filter_map(|(equipment, (item_id, cells, first_step))| {
            if invalid.contains(&equipment) {
                return None;
            }
            let definition = catalog
                .items
                .get(&item_id)
                .expect("compact module candidate must retain its catalog definition");
            let shape = catalog
                .shapes
                .get(&definition.geometry)
                .expect("catalog-validated module must have a shape");
            let first_step = first_step?;
            let expected = shape
                .cells
                .iter()
                .map(|cell| EquipmentGridCell {
                    row: first_step.row + cell.row,
                    column: first_step.column + cell.column,
                })
                .collect::<HashSet<_>>();
            if Some(cells.len() as u32) != definition.grid || cells != expected {
                return None;
            }
            Some(ParsedEmptyCurtainCompactModulePlacement {
                equipment,
                item_id,
                row: first_step.row,
                column: first_step.column,
            })
        })
        .collect::<Vec<_>>();
    placements.sort_by_key(|placement| (placement.equipment.solt, placement.equipment.serial));
    placements
}

fn parse_empty_curtain_module_placements(
    slots: &[ParsedEquipmentSlot],
    known_items: &HashMap<HtItemNetId, EmptyCurtainItem>,
    catalog: &EquipmentCatalog,
) -> Option<Vec<ParsedEmptyCurtainModulePlacement>> {
    if slots.len() != EMPTY_CURTAIN_GRID_CELLS {
        return None;
    }
    let bit_shift = slots.first()?.bit_shift;
    let mut cells = HashSet::with_capacity(EMPTY_CURTAIN_GRID_CELLS);
    let mut modules =
        HashMap::<HtItemNetId, (&str, HashSet<EquipmentGridCell>, Option<EquipmentGridCell>)>::new(
        );
    for slot in slots {
        if slot.bit_shift != bit_shift
            || !(0..EMPTY_CURTAIN_GRID_SIDE).contains(&slot.row)
            || !(0..EMPTY_CURTAIN_GRID_SIDE).contains(&slot.column)
            || !cells.insert((slot.row, slot.column))
        {
            return None;
        }
        let id = HtItemNetId {
            solt: slot.equip_net_id.solt,
            serial: slot.equip_net_id.serial,
        };
        if id.is_zero() {
            if slot.equipment_id != "None" || !matches!(slot.state, -1 | 0) || slot.first_step {
                return None;
            }
            continue;
        }
        if !valid_item_net_id(id) || slot.state != 1 || slot.equipment_id == "None" {
            return None;
        }
        let definition = catalog.items.get(&slot.equipment_id)?;
        if definition.kind != EquipmentKind::Module
            || known_items
                .get(&id)
                .is_some_and(|item| item.item_id != slot.equipment_id)
        {
            return None;
        }
        // A grid update can arrive before that item's detailed inventory record. The exact
        // 49-cell shape remains authoritative as long as its embedded catalog ID and geometry
        // are self-consistent; known items still receive the stronger UID-to-item cross-check.
        let entry = modules
            .entry(id)
            .or_insert_with(|| (slot.equipment_id.as_str(), HashSet::new(), None));
        if entry.0 != slot.equipment_id.as_str()
            || !entry.1.insert(EquipmentGridCell {
                row: slot.row,
                column: slot.column,
            })
        {
            return None;
        }
        if slot.first_step
            && entry
                .2
                .replace(EquipmentGridCell {
                    row: slot.row,
                    column: slot.column,
                })
                .is_some()
        {
            return None;
        }
    }
    for (item_id, occupied, first_step) in modules.values() {
        let definition = catalog
            .items
            .get(*item_id)
            .expect("validated module definitions must remain present");
        let shape = catalog
            .shapes
            .get(&definition.geometry)
            .expect("catalog-validated module must have a shape");
        let Some(first_step) = first_step else {
            return None;
        };
        let expected = shape
            .cells
            .iter()
            .map(|cell| EquipmentGridCell {
                row: first_step.row + cell.row,
                column: first_step.column + cell.column,
            })
            .collect::<HashSet<_>>();
        if occupied != &expected {
            return None;
        }
    }
    let mut placements = modules
        .into_iter()
        .map(|(equipment, (item_id, _, position))| {
            let position = position.expect("validated modules must have one first cell");
            ParsedEmptyCurtainModulePlacement {
                equipment,
                item_id: item_id.to_owned(),
                row: position.row,
                column: position.column,
            }
        })
        .collect::<Vec<_>>();
    placements.sort_by_key(|placement| (placement.equipment.solt, placement.equipment.serial));
    Some(placements)
}

fn contains_inventory_bytes(data: &[u8], data_bit_len: usize, expected: &[u8]) -> bool {
    let expected_bit_len = expected.len().saturating_mul(8);
    if expected_bit_len == 0 || expected_bit_len > data_bit_len {
        return false;
    }
    (0..=data_bit_len - expected_bit_len).any(|bit_offset| {
        expected.iter().enumerate().all(|(byte_index, byte)| {
            (0..8).all(|bit_index| {
                let source = bit_offset + byte_index * 8 + bit_index;
                ((data[source / 8] >> (source % 8)) & 1) == ((byte >> bit_index) & 1)
            })
        })
    })
}

pub(crate) fn parse_empty_curtain_equipment_snapshot(
    data: &[u8],
    data_bit_len: usize,
    character_ids: &HashMap<HtItemNetId, u32>,
    known_items: &HashMap<HtItemNetId, EmptyCurtainItem>,
    catalog: &EquipmentCatalog,
) -> Option<ParsedEmptyCurtainEquipmentSnapshot> {
    if data_bit_len > data.len().saturating_mul(8) || character_ids.len() != 1 {
        return None;
    }
    let (&character_net_id, &character_id) = character_ids.iter().next()?;
    let slots = parse_equipment_slots(data);
    if !slots.is_empty() {
        return Some(ParsedEmptyCurtainEquipmentSnapshot::Modules {
            character_net_id,
            character_id,
            placements: parse_empty_curtain_module_placements(&slots, known_items, catalog)?,
        });
    }
    // Core syncs carry one validated character record and the serialized slot type marker,
    // but not the tagged 7x7 module grid. A missing known core UID is the empty-slot snapshot.
    if !contains_inventory_bytes(data, data_bit_len, EMPTY_CURTAIN_CORE_SLOT_ANCHOR) {
        return None;
    }

    let mut core_ids = HashSet::new();
    for bit_offset in 0..=data_bit_len.saturating_sub(64) {
        let mut reader = InventoryBitReader::at(data, data_bit_len, bit_offset)?;
        let id = reader.read_item_net_id()?;
        let Some(item) = known_items.get(&id) else {
            continue;
        };
        let Some(definition) = catalog.items.get(&item.item_id) else {
            continue;
        };
        if definition.kind == EquipmentKind::Core {
            core_ids.insert(id);
        }
    }
    if core_ids.len() > 1 {
        return None;
    }
    Some(ParsedEmptyCurtainEquipmentSnapshot::Core {
        character_net_id,
        character_id,
        item_id: core_ids.into_iter().next(),
    })
}

fn parse_empty_curtain_item_at(
    mut reader: InventoryBitReader<'_>,
    catalog: &EquipmentCatalog,
) -> Option<(EmptyCurtainItem, usize)> {
    let item_id = reader.read_dynamic_name()?;
    let definition = catalog.items.get(&item_id)?;
    let id = reader.read_item_net_id()?;
    if !valid_item_net_id(id) || reader.read_i64()? != 1 {
        return None;
    }

    let _durable = reader.read_i32()?;
    let create_time = reader.read_i64()?;
    if create_time < 0
        || reader.read_u16()? != 0
        || reader.read_u16()? != 0
        || reader.read_u16()? != 1
    {
        return None;
    }

    let level = reader.read_i32()?;
    let level = u32::try_from(level).ok()?;
    if level > definition.max_level {
        return None;
    }

    let character_id = reader.read_item_net_id()?;
    let character_net_id = if character_id.is_zero() {
        None
    } else if valid_item_net_id(character_id) {
        Some(character_id)
    } else {
        return None;
    };
    if reader.read_i32()? < 0 {
        return None;
    }
    let locked = reader.read_bool()?;
    let discarded = reader.read_bool()?;
    if reader.read_u16()? != 0 || usize::from(reader.read_u16()?) != definition.main_count {
        return None;
    }

    let mut main_stats = Vec::with_capacity(definition.main_count);
    for _ in 0..definition.main_count {
        let property = reader.read_dynamic_name()?;
        if !catalog.attributes.contains_key(&property) {
            return None;
        }
        let value = catalog.main_stat_value(definition, &property, level)?;
        if !value.is_finite() {
            return None;
        }
        main_stats.push(EquipmentStat { property, value });
    }

    if usize::from(reader.read_u16()?) != definition.sub_count {
        return None;
    }
    let mut sub_stats = Vec::with_capacity(definition.sub_count);
    for _ in 0..definition.sub_count {
        let property = reader.read_dynamic_name()?;
        if !catalog.attributes.contains_key(&property) {
            return None;
        }
        let value = reader.read_f32()?;
        if !value.is_finite() || value <= 0.0 || value > MAX_INVENTORY_STAT_VALUE {
            return None;
        }
        sub_stats.push(EquipmentStat { property, value });
    }

    let shortcut_timer = reader.read_f32()?;
    let _temporary = reader.read_bool()?;
    let equipped_tail = reader.read_i32()?;
    if !shortcut_timer.is_finite() || shortcut_timer < 0.0 || !matches!(equipped_tail, 0 | 1) {
        return None;
    }
    reader.read_fstring(MAX_INVENTORY_EXTRA_JSON_LENGTH)?;

    Some((
        EmptyCurtainItem {
            id,
            item_id,
            level,
            main_stats,
            sub_stats,
            locked,
            discarded,
            character_net_id,
            equipped_character_id: None,
            equipped_placement: None,
        },
        reader.cursor,
    ))
}

pub fn parse_empty_curtain_items(
    data: &[u8],
    data_bit_len: usize,
    catalog: &EquipmentCatalog,
) -> Vec<EmptyCurtainItem> {
    if data_bit_len > data.len().saturating_mul(8) {
        return Vec::new();
    }
    let mut items = HashMap::new();
    let mut bit_offset = 0;
    while bit_offset < data_bit_len {
        let Some(reader) = InventoryBitReader::at(data, data_bit_len, bit_offset) else {
            break;
        };
        if let Some((item, next_offset)) = parse_empty_curtain_item_at(reader, catalog) {
            if !items.contains_key(&item.id) && items.len() >= MAX_EMPTY_CURTAIN_ITEMS {
                break;
            }
            items.insert(item.id, item);
            bit_offset = next_offset.max(bit_offset + 1);
        } else {
            bit_offset += 1;
        }
    }
    let mut items: Vec<_> = items.into_values().collect();
    items.sort_by_key(|item| (item.id.solt, item.id.serial));
    items
}

pub(crate) fn parse_empty_curtain_item_additions(
    data: &[u8],
    data_bit_len: usize,
    catalog: &EquipmentCatalog,
) -> Vec<EmptyCurtainItem> {
    const NOTIFY_ITEM_ADD: u64 = 3;
    const NOTIFICATION_HEADER_BITS: usize = 24;

    if data_bit_len > data.len().saturating_mul(8) || data_bit_len < NOTIFICATION_HEADER_BITS {
        return Vec::new();
    }

    let mut best = Vec::new();
    for bit_offset in 0..=data_bit_len - NOTIFICATION_HEADER_BITS {
        let Some(mut reader) = InventoryBitReader::at(data, data_bit_len, bit_offset) else {
            break;
        };
        if reader.read_bits(7) != Some(NOTIFY_ITEM_ADD) {
            continue;
        }
        let Some(item_count) = reader.read_bits(17).map(|count| count as usize) else {
            continue;
        };
        if item_count == 0 || item_count > MAX_EMPTY_CURTAIN_ITEMS {
            continue;
        }

        let mut additions = Vec::new();
        for _ in 0..item_count {
            let Some((item, next_offset)) = parse_empty_curtain_item_at(reader, catalog) else {
                break;
            };
            additions.push(item);
            reader = InventoryBitReader::at(data, data_bit_len, next_offset)
                .expect("a parsed inventory item must end inside the packet");
        }
        if additions.len() == item_count {
            return additions;
        }
        if additions.len() > best.len() {
            best = additions;
        }
    }
    best
}

pub(crate) fn parse_empty_curtain_item_removals(
    data: &[u8],
    data_bit_len: usize,
    catalog: &EquipmentCatalog,
) -> Vec<(HtItemNetId, String)> {
    const NOTIFY_ITEM_REMOVE: u64 = 2;
    const NOTIFICATION_HEADER_BITS: usize = 24;

    if data_bit_len > data.len().saturating_mul(8) || data_bit_len < NOTIFICATION_HEADER_BITS {
        return Vec::new();
    }

    for bit_offset in 0..=data_bit_len - NOTIFICATION_HEADER_BITS {
        let Some(mut reader) = InventoryBitReader::at(data, data_bit_len, bit_offset) else {
            break;
        };
        if reader.read_bits(7) != Some(NOTIFY_ITEM_REMOVE) {
            continue;
        }
        let Some(item_count) = reader.read_bits(17).map(|count| count as usize) else {
            continue;
        };
        if item_count == 0 || item_count > MAX_EMPTY_CURTAIN_ITEMS {
            continue;
        }

        let mut removals = Vec::new();
        for _ in 0..item_count {
            let Some((item, next_offset)) = parse_empty_curtain_item_at(reader, catalog) else {
                break;
            };
            removals.push((item.id, item.item_id));
            reader = InventoryBitReader::at(data, data_bit_len, next_offset)
                .expect("a parsed inventory item must end inside the packet");
        }
        if removals.len() == item_count {
            return removals;
        }
    }
    Vec::new()
}

pub fn validate_empty_curtain_snapshot(
    items: &[EmptyCurtainItem],
    catalog: &EquipmentCatalog,
) -> bool {
    if items.len() > MAX_EMPTY_CURTAIN_ITEMS {
        return false;
    }
    let mut ids = HashSet::with_capacity(items.len());
    items.iter().all(|item| {
        let Some(definition) = catalog.items.get(&item.item_id) else {
            return false;
        };
        if !valid_item_net_id(item.id)
            || !ids.insert(item.id)
            || item.level > definition.max_level
            || item.main_stats.len() != definition.main_count
            || item.sub_stats.len() != definition.sub_count
            || item.main_stats.len() + item.sub_stats.len() > EMPTY_CURTAIN_MAX_STAT_ROWS
            || item
                .character_net_id
                .is_some_and(|character_id| !valid_item_net_id(character_id))
            || item.equipped_character_id == Some(0)
            || (item.equipped_character_id.is_some() && item.character_net_id.is_none())
            || (definition.kind == EquipmentKind::Core && item.equipped_placement.is_some())
            || item.equipped_placement.is_some_and(|placement| {
                item.character_net_id.is_none()
                    || !(0..EMPTY_CURTAIN_GRID_SIDE).contains(&placement.row)
                    || !(0..EMPTY_CURTAIN_GRID_SIDE).contains(&placement.column)
            })
        {
            return false;
        }

        item.main_stats.iter().all(|stat| {
            catalog.attributes.contains_key(&stat.property)
                && catalog.main_stat_value(definition, &stat.property, item.level)
                    == Some(stat.value)
        }) && item.sub_stats.iter().all(|stat| {
            catalog.attributes.contains_key(&stat.property)
                && stat.value.is_finite()
                && stat.value > 0.0
                && stat.value <= MAX_INVENTORY_STAT_VALUE
        })
    })
}

fn parse_damage_record_at(
    data: &[u8],
    byte_offset: usize,
    bit_shift: u8,
) -> Option<ParsedDamageRecord> {
    let mut fields = [Field::default(); RECORD_FIELD_COUNT];
    let mut bit_cursor = 0;
    for (index, (expected_type, expected_length)) in RECORD_PREFIX_FIELD_TYPES
        .into_iter()
        .zip(RECORD_PREFIX_FIELD_LENGTHS)
        .enumerate()
    {
        let (field_type, field, consumed) = read_field(data, byte_offset, bit_shift, bit_cursor)?;
        if field_type != expected_type || field.len != expected_length {
            return None;
        }
        bit_cursor += consumed;
        fields[index] = field;
    }

    let (first_state_type, first_state, consumed) =
        read_field(data, byte_offset, bit_shift, bit_cursor)?;
    let state_encoding = match (first_state_type, first_state.len) {
        (6, 4) => DamageRecordEncoding::LegacyInt32,
        (8, 1) => DamageRecordEncoding::BoolAndEnums,
        _ => return None,
    };
    bit_cursor += consumed;
    fields[6] = first_state;

    let (state_types, state_lengths) = match state_encoding {
        DamageRecordEncoding::LegacyInt32 => (LEGACY_STATE_FIELD_TYPES, LEGACY_STATE_FIELD_LENGTHS),
        DamageRecordEncoding::BoolAndEnums => {
            (BOOL_ENUM_STATE_FIELD_TYPES, BOOL_ENUM_STATE_FIELD_LENGTHS)
        }
    };
    for state_index in 1..state_types.len() {
        let (field_type, field, consumed) = read_field(data, byte_offset, bit_shift, bit_cursor)?;
        if field_type != state_types[state_index] || field.len != state_lengths[state_index] {
            return None;
        }
        bit_cursor += consumed;
        fields[6 + state_index] = field;
    }

    let (trailing_type, trailing_field, _) = read_field(data, byte_offset, bit_shift, bit_cursor)?;
    if trailing_type != RECORD_TRAILING_FIELD_TYPE
        || trailing_field.len != RECORD_TRAILING_FIELD_LENGTH
    {
        return None;
    }
    fields[9] = trailing_field;

    let damage = f32_field(&fields[0])?;
    let target_hp_before = f32_field(&fields[1])?;
    let target_max_hp = f32_field(&fields[2])?;
    let damage_time = f64_field(&fields[3])?;
    let world_time = f32_field(&fields[4])?;
    let repeated_damage = f32_field(&fields[5])?;
    let state_flags = match state_encoding {
        DamageRecordEncoding::LegacyInt32 => [
            i32_field(&fields[6])?,
            i32_field(&fields[7])?,
            i32_field(&fields[8])?,
        ],
        DamageRecordEncoding::BoolAndEnums => [
            match fields[6].raw[0] {
                0 => 0,
                1 => 1,
                _ => return None,
            },
            i32_field(&fields[7])?,
            i32_field(&fields[8])?,
        ],
    };
    let trailing_value = f32_field(&fields[9])?;

    if !damage.is_finite()
        || !(MIN_DAMAGE..=MAX_COMBAT_VALUE).contains(&damage)
        || !target_hp_before.is_finite()
        || target_hp_before < 0.0
        || !target_max_hp.is_finite()
        || target_max_hp <= 0.0
        || !damage_time.is_finite()
        || damage_time < 0.0
        || !world_time.is_finite()
        || world_time < 0.0
        || !trailing_value.is_finite()
    {
        return None;
    }

    let tolerance = 0.01_f32.max(damage.abs() * 1e-6);
    if (damage - repeated_damage).abs() > tolerance {
        return None;
    }

    Some(ParsedDamageRecord {
        damage,
        target_hp_before,
        target_max_hp,
        damage_time,
        world_time,
        repeated_damage,
        state_flags,
        trailing_value,
        encoding: state_encoding,
        byte_offset: byte_offset + 5,
        bit_shift,
    })
}

pub fn damage_record_encoding_at(
    data: &[u8],
    damage_byte_offset: usize,
    bit_shift: u8,
) -> Option<DamageRecordEncoding> {
    let record_start = damage_byte_offset.checked_sub(5)?;
    let record = parse_damage_record_at(data, record_start, bit_shift)?;
    (record.byte_offset == damage_byte_offset).then_some(record.encoding)
}

pub fn parse_damage_records(data: &[u8]) -> Vec<ParsedDamageRecord> {
    // Each (byte_offset, bit_shift) pair is visited exactly once, so no dedup set is needed.
    let mut records = Vec::new();
    for byte_offset in 0..data.len() {
        for bit_shift in 0..8_u8 {
            if let Some(record) = parse_damage_record_at(data, byte_offset, bit_shift) {
                records.push(record);
            }
        }
    }
    records
}

pub fn parse_current_hp_updates(data: &[u8]) -> Vec<ParsedCurrentHpUpdate> {
    let mut updates = Vec::new();
    for byte_offset in 0..data.len() {
        for bit_shift in 0..8_u8 {
            let mut decoded = [0; CURRENT_HP_PREFIX_LENGTH + 4];
            if decode_shifted_into(data, byte_offset, bit_shift, 0, &mut decoded).is_none() {
                continue;
            }
            let prefix = &decoded[..CURRENT_HP_PREFIX_LENGTH];
            if prefix[1..7] != [0, 0, 0xe0, 0x4f, 0x33, 0x33]
                || prefix[8] != 0x0f
                || prefix[11..16] != [0, 0, 0, 0, 0x24]
            {
                continue;
            }
            let current_hp =
                f32::from_le_bytes([decoded[16], decoded[17], decoded[18], decoded[19]]);
            if !current_hp.is_finite()
                || !(0.0..=MAX_PLAUSIBLE_CURRENT_HP_UPDATE).contains(&current_hp)
            {
                continue;
            }
            updates.push(ParsedCurrentHpUpdate {
                current_hp,
                byte_offset: byte_offset + CURRENT_HP_PREFIX_LENGTH,
                bit_shift,
            });
        }
    }
    updates
}

fn parse_boss_hp_update_at(
    data: &[u8],
    byte_offset: usize,
    bit_shift: u8,
    zero_prefix_handles: Option<&HashSet<[u8; CLIENT_FIGHT_TARGET_WIRE_BYTES]>>,
) -> Option<ParsedBossHpUpdate> {
    let mut decoded = [0; BOSS_HP_PREFIX_LENGTH + 4];
    decode_shifted_into(data, byte_offset, bit_shift, 0, &mut decoded)?;
    let record_bit_offset = byte_offset
        .checked_mul(8)?
        .checked_add(usize::from(bit_shift))?;
    let target_handle = parse_client_fight_target_wire_identity(
        data,
        record_bit_offset.checked_add(CLIENT_FIGHT_DATA_ARRAY_HEADER_BITS)?,
    )?;
    let prefix = &decoded[..BOSS_HP_PREFIX_HEAD.len()];
    let encoded_count = u16::from_le_bytes([prefix[5], prefix[6]]);
    let element_count = usize::from(encoded_count >> 5);
    let tagged_prefix = prefix[..5] == BOSS_HP_PREFIX_HEAD[..5]
        && encoded_count & 0x1f == 0
        && (1..=MAX_CLIENT_FIGHT_DATA_ELEMENTS).contains(&element_count)
        && prefix[7] & 0x1f == 0;
    let continued_prefix = prefix.iter().all(|byte| *byte == 0)
        && zero_prefix_handles.is_some_and(|handles| handles.contains(&target_handle));
    if !tagged_prefix && !continued_prefix {
        return None;
    }
    let current_hp = f32::from_le_bytes(decoded[BOSS_HP_PREFIX_LENGTH..].try_into().ok()?);
    if !current_hp.is_finite() || !(0.0..=MAX_COMBAT_VALUE).contains(&current_hp) {
        return None;
    }
    Some(ParsedBossHpUpdate {
        target_handle,
        current_hp,
        byte_offset: byte_offset + BOSS_HP_PREFIX_LENGTH,
        bit_shift,
    })
}

fn parse_client_fight_target_wire_identity(
    data: &[u8],
    bit_offset: usize,
) -> Option<[u8; CLIENT_FIGHT_TARGET_WIRE_BYTES]> {
    let mut identity = [0; CLIENT_FIGHT_TARGET_WIRE_BYTES];
    decode_shifted_into(
        data,
        bit_offset / 8,
        (bit_offset % 8) as u8,
        0,
        &mut identity,
    )?;
    identity[CLIENT_FIGHT_TARGET_WIRE_BYTES - 1] &= 0x07;

    // SDK `FCharacterForNet::NetSerialize` writes a 128-bit opaque reference
    // followed by the compact instance fields. The SDK-confirmed variants use
    // 0x07, 0x0f, or 0x17 at byte 16; requiring every remaining reserved byte
    // to be zero prevents unrelated shifted data from entering the
    // authoritative per-target HP map.
    if !identity[..16].iter().any(|byte| *byte != 0)
        || !matches!(identity[16], 0x07 | 0x0f | 0x17)
        || identity[17..20].iter().any(|byte| *byte != 0)
        || identity[21..].iter().any(|byte| *byte != 0)
    {
        return None;
    }
    Some(identity)
}

fn parse_compact_client_fight_target_identity(
    data: &[u8],
    bit_offset: usize,
) -> Option<[u8; CLIENT_FIGHT_TARGET_WIRE_BYTES]> {
    let mut compact = [0_u8; COMPACT_CLIENT_FIGHT_IDENTITY_BYTES];
    decode_shifted_into(
        data,
        bit_offset / 8,
        (bit_offset % 8) as u8,
        0,
        &mut compact,
    )?;
    if !compact[..16].iter().any(|byte| *byte != 0) || compact[20..].iter().any(|byte| *byte != 0) {
        return None;
    }
    let mut identity = [0_u8; CLIENT_FIGHT_TARGET_WIRE_BYTES];
    identity[..COMPACT_CLIENT_FIGHT_IDENTITY_BYTES].copy_from_slice(&compact);
    Some(identity)
}

/// Decodes SDK `ClientSetReplicatedTargetData.ClientFightDataArray` directly
/// from the external transport payload.
///
/// Each bounded array element contains the exact 227-bit
/// `FCharacterForNet::NetSerialize` identity and `ClientRepFightData.fCurHealth`.
/// A zero-leading wrapper is accepted only for the SDK-confirmed terminal
/// `DeadState == 2 && fCurHealth == 0` shape, avoiding heuristic target claims.
fn parse_bitpacked_client_fight_target_updates(data: &[u8]) -> Vec<ParsedBossHpUpdate> {
    const MIN_RECORD_BITS: usize = CLIENT_FIGHT_DATA_DEAD_STATE_BIT_OFFSET + 32;

    let data_bits = data.len().saturating_mul(8);
    if data_bits < MIN_RECORD_BITS {
        return Vec::new();
    }

    let mut updates = Vec::new();
    for record_bit_offset in 0..=data_bits - MIN_RECORD_BITS {
        let mut prefix = [0; BOSS_HP_PREFIX_HEAD.len()];
        if decode_shifted_into(
            data,
            record_bit_offset / 8,
            (record_bit_offset % 8) as u8,
            0,
            &mut prefix,
        )
        .is_none()
        {
            continue;
        }
        let encoded_count = u16::from_le_bytes([prefix[5], prefix[6]]);
        let element_count = usize::from(encoded_count >> 5);
        let normal_array = prefix[..5] == BOSS_HP_PREFIX_HEAD[..5];
        let terminal_death = prefix[..5].iter().all(|byte| *byte == 0) && element_count == 1;
        if encoded_count & 0x1f != 0
            || !(1..=MAX_CLIENT_FIGHT_DATA_ELEMENTS).contains(&element_count)
            || prefix[7] & 0x1f != 0
            || (!normal_array && !terminal_death)
        {
            continue;
        }
        let Some(last_record_bit_offset) = (element_count - 1)
            .checked_mul(CLIENT_FIGHT_DATA_ELEMENT_BITS)
            .and_then(|bits| record_bit_offset.checked_add(bits))
        else {
            continue;
        };
        if last_record_bit_offset
            .checked_add(MIN_RECORD_BITS)
            .is_none_or(|end| end > data_bits)
        {
            continue;
        }

        let mut candidate = Vec::with_capacity(element_count);
        for element_index in 0..element_count {
            let element_bit_offset =
                record_bit_offset + element_index * CLIENT_FIGHT_DATA_ELEMENT_BITS;
            let Some(target_handle) = parse_client_fight_target_wire_identity(
                data,
                element_bit_offset + CLIENT_FIGHT_DATA_ARRAY_HEADER_BITS,
            ) else {
                candidate.clear();
                break;
            };
            let mut current_hp_bytes = [0; 4];
            let mut dead_state_bytes = [0; 4];
            if decode_shifted_into(
                data,
                element_bit_offset / 8,
                (element_bit_offset % 8) as u8,
                CLIENT_FIGHT_DATA_HP_BIT_OFFSET,
                &mut current_hp_bytes,
            )
            .is_none()
                || decode_shifted_into(
                    data,
                    element_bit_offset / 8,
                    (element_bit_offset % 8) as u8,
                    CLIENT_FIGHT_DATA_DEAD_STATE_BIT_OFFSET,
                    &mut dead_state_bytes,
                )
                .is_none()
            {
                candidate.clear();
                break;
            }
            let current_hp = f32::from_le_bytes(current_hp_bytes);
            let dead_state = u32::from_le_bytes(dead_state_bytes);
            if !current_hp.is_finite()
                || !(0.0..=MAX_COMBAT_VALUE).contains(&current_hp)
                || dead_state > 2
                || (terminal_death && (current_hp != 0.0 || dead_state != 2))
            {
                candidate.clear();
                break;
            }
            let hp_bit_offset = element_bit_offset + CLIENT_FIGHT_DATA_HP_BIT_OFFSET;
            candidate.push(ParsedBossHpUpdate {
                target_handle,
                current_hp,
                byte_offset: hp_bit_offset / 8,
                bit_shift: (hp_bit_offset % 8) as u8,
            });
        }
        if candidate.len() == element_count {
            updates.extend(candidate);
        }
    }
    updates
}

/// Decodes the current SDK `ClientRepFightData` wire layout.
///
/// `FCharacterForNet::NetSerialize` now writes the 16-byte opaque actor value,
/// the 4-byte instance discriminator, and 8 reserved zero bytes directly before
/// `fCurHealth`. The surrounding array prefix and the remaining SDK fields are
/// all required so an arbitrary UUID-like byte run cannot create a target.
fn parse_compact_client_fight_target_updates(data: &[u8]) -> Vec<ParsedBossHpUpdate> {
    #[derive(Clone)]
    struct Candidate {
        update: ParsedBossHpUpdate,
        actor_value: [u8; 16],
        declared_element_count: Option<usize>,
    }

    if data.len() < COMPACT_CLIENT_FIGHT_RECORD_BYTES {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    for bit_shift in 0..8_u8 {
        for byte_offset in 0..=data.len() - COMPACT_CLIENT_FIGHT_RECORD_BYTES {
            let mut decoded = [0_u8; COMPACT_CLIENT_FIGHT_RECORD_BYTES];
            if decode_shifted_into(data, byte_offset, bit_shift, 0, &mut decoded).is_none() {
                continue;
            }
            let prefix = &decoded[..8];
            let encoded_count = u16::from_le_bytes([prefix[5], prefix[6]]);
            let element_count = usize::from(encoded_count >> 5);
            let normal_array = prefix[..5] == BOSS_HP_PREFIX_HEAD[..5]
                && encoded_count & 0x1f == 0
                && (1..=MAX_CLIENT_FIGHT_DATA_ELEMENTS).contains(&element_count)
                && prefix[7] & 0x1f == 0;
            let terminal_array = prefix[..5].iter().all(|byte| *byte == 0)
                && encoded_count == 0x20
                && prefix[7] == 0;
            let continuation = prefix.iter().all(|byte| *byte == 0);
            if !normal_array && !terminal_array && !continuation {
                continue;
            }

            let identity = &decoded[8..8 + COMPACT_CLIENT_FIGHT_IDENTITY_BYTES];
            let mut actor_value = [0_u8; 16];
            actor_value.copy_from_slice(&identity[..16]);
            if !actor_value.iter().any(|byte| *byte != 0)
                || identity[20..].iter().any(|byte| *byte != 0)
            {
                continue;
            }
            let current_hp =
                f32::from_le_bytes([decoded[36], decoded[37], decoded[38], decoded[39]]);
            let dead_state =
                u32::from_le_bytes([decoded[40], decoded[41], decoded[42], decoded[43]]);
            let shield_damage =
                f32::from_le_bytes([decoded[44], decoded[45], decoded[46], decoded[47]]);
            let lock_target =
                u32::from_le_bytes([decoded[48], decoded[49], decoded[50], decoded[51]]);
            if !current_hp.is_finite()
                || !(0.0..=MAX_COMBAT_VALUE).contains(&current_hp)
                || dead_state > 2
                || !shield_damage.is_finite()
                || !(0.0..=MAX_COMBAT_VALUE).contains(&shield_damage)
                || lock_target > 1
                || (terminal_array && (current_hp != 0.0 || dead_state != 2))
            {
                continue;
            }

            let mut target_handle = [0_u8; CLIENT_FIGHT_TARGET_WIRE_BYTES];
            target_handle[..COMPACT_CLIENT_FIGHT_IDENTITY_BYTES].copy_from_slice(identity);
            candidates.push(Candidate {
                update: ParsedBossHpUpdate {
                    target_handle,
                    current_hp,
                    byte_offset: byte_offset + 36,
                    bit_shift,
                },
                actor_value,
                declared_element_count: (normal_array || terminal_array).then_some(element_count),
            });
        }
    }

    let actor_candidate_counts = candidates.iter().fold(
        HashMap::<[u8; 16], usize>::new(),
        |mut counts, candidate| {
            *counts.entry(candidate.actor_value).or_default() += 1;
            counts
        },
    );
    let declared_actor_values = candidates
        .iter()
        .filter(|candidate| {
            candidate.declared_element_count.is_some_and(|declared| {
                actor_candidate_counts
                    .get(&candidate.actor_value)
                    .is_some_and(|actual| *actual >= declared)
            })
        })
        .map(|candidate| candidate.actor_value)
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|candidate| {
            candidate.declared_element_count.is_some_and(|declared| {
                actor_candidate_counts
                    .get(&candidate.actor_value)
                    .is_some_and(|actual| *actual >= declared)
            }) || declared_actor_values.contains(&candidate.actor_value)
        })
        .filter_map(|candidate| {
            seen.insert((
                candidate.update.target_handle,
                candidate.update.current_hp.to_bits(),
            ))
            .then_some(candidate.update)
        })
        .collect()
}

pub fn parse_client_fight_target_updates(data: &[u8]) -> Vec<ParsedBossHpUpdate> {
    let compact = parse_compact_client_fight_target_updates(data);
    if compact.is_empty() {
        parse_bitpacked_client_fight_target_updates(data)
    } else {
        compact
    }
}

/// Decodes authoritative server damage settlements from
/// `ClientSetReplicatedTargetData.ClientFightDataArray`.
///
/// The current SDK serializes every ordinary damage row as two parameter
/// wrappers. Type `0x06` contains a negative little-endian `i32` damage value;
/// type `0x1f` contains the secondary settlement marker. Requiring the complete
/// array, both wrapper types and both four-byte payloads prevents a shifted
/// integer elsewhere in an untrusted UDP payload from becoming damage.
pub fn parse_server_damage_settlements(data: &[u8]) -> Vec<ParsedServerDamageSettlement> {
    const REQUIRED_ELEMENT_BITS: usize = CLIENT_FIGHT_DATA_SECOND_WRAPPER_VALUE_BIT_OFFSET + 32;

    let data_bits = data.len().saturating_mul(8);
    if data_bits < REQUIRED_ELEMENT_BITS {
        return Vec::new();
    }

    let mut settlements = Vec::new();
    let mut seen = HashSet::new();
    for record_bit_offset in 0..=data_bits - REQUIRED_ELEMENT_BITS {
        let mut prefix = [0_u8; BOSS_HP_PREFIX_HEAD.len()];
        if decode_shifted_into(
            data,
            record_bit_offset / 8,
            (record_bit_offset % 8) as u8,
            0,
            &mut prefix,
        )
        .is_none()
        {
            continue;
        }
        let encoded_count = u16::from_le_bytes([prefix[5], prefix[6]]);
        let element_count = usize::from(encoded_count >> 5);
        if prefix[..5] != BOSS_HP_PREFIX_HEAD[..5]
            || encoded_count & 0x1f != 0
            || !(1..=MAX_CLIENT_FIGHT_DATA_ELEMENTS).contains(&element_count)
            || prefix[7] & 0x1f != 0
        {
            continue;
        }
        let Some(last_element_bit_offset) = (element_count - 1)
            .checked_mul(CLIENT_FIGHT_DATA_ELEMENT_BITS)
            .and_then(|bits| record_bit_offset.checked_add(bits))
        else {
            continue;
        };
        if last_element_bit_offset
            .checked_add(REQUIRED_ELEMENT_BITS)
            .is_none_or(|end| end > data_bits)
        {
            continue;
        }

        let mut candidate = Vec::with_capacity(element_count);
        let mut candidate_valid = true;
        for element_index in 0..element_count {
            let element_bit_offset =
                record_bit_offset + element_index * CLIENT_FIGHT_DATA_ELEMENT_BITS;
            let Some(target_handle) = parse_client_fight_target_wire_identity(
                data,
                element_bit_offset + CLIENT_FIGHT_DATA_ARRAY_HEADER_BITS,
            ) else {
                candidate_valid = false;
                break;
            };
            let mut current_hp_bytes = [0_u8; 4];
            let mut dead_state_bytes = [0_u8; 4];
            let mut wrapper_count_bytes = [0_u8; 4];
            let mut first_type = [0_u8; 1];
            let mut first_length_bytes = [0_u8; 4];
            let mut first_value_bytes = [0_u8; 4];
            let mut second_type = [0_u8; 1];
            let mut second_length_bytes = [0_u8; 4];
            let mut second_value_bytes = [0_u8; 4];
            let fields = [
                (
                    CLIENT_FIGHT_DATA_HP_BIT_OFFSET,
                    current_hp_bytes.as_mut_slice(),
                ),
                (
                    CLIENT_FIGHT_DATA_DEAD_STATE_BIT_OFFSET,
                    dead_state_bytes.as_mut_slice(),
                ),
                (
                    CLIENT_FIGHT_DATA_WRAPPER_COUNT_BIT_OFFSET,
                    wrapper_count_bytes.as_mut_slice(),
                ),
                (
                    CLIENT_FIGHT_DATA_FIRST_WRAPPER_TYPE_BIT_OFFSET,
                    first_type.as_mut_slice(),
                ),
                (
                    CLIENT_FIGHT_DATA_FIRST_WRAPPER_LENGTH_BIT_OFFSET,
                    first_length_bytes.as_mut_slice(),
                ),
                (
                    CLIENT_FIGHT_DATA_FIRST_WRAPPER_VALUE_BIT_OFFSET,
                    first_value_bytes.as_mut_slice(),
                ),
                (
                    CLIENT_FIGHT_DATA_SECOND_WRAPPER_TYPE_BIT_OFFSET,
                    second_type.as_mut_slice(),
                ),
                (
                    CLIENT_FIGHT_DATA_SECOND_WRAPPER_LENGTH_BIT_OFFSET,
                    second_length_bytes.as_mut_slice(),
                ),
                (
                    CLIENT_FIGHT_DATA_SECOND_WRAPPER_VALUE_BIT_OFFSET,
                    second_value_bytes.as_mut_slice(),
                ),
            ];
            if fields.into_iter().any(|(relative_bit_offset, output)| {
                decode_shifted_into(
                    data,
                    element_bit_offset / 8,
                    (element_bit_offset % 8) as u8,
                    relative_bit_offset,
                    output,
                )
                .is_none()
            }) {
                candidate_valid = false;
                break;
            }

            let current_hp = f32::from_le_bytes(current_hp_bytes);
            let dead_state = u32::from_le_bytes(dead_state_bytes);
            let wrapper_count = u32::from_le_bytes(wrapper_count_bytes);
            let first_length = u32::from_le_bytes(first_length_bytes);
            let second_length = u32::from_le_bytes(second_length_bytes);
            let signed_damage = i32::from_le_bytes(first_value_bytes);
            if signed_damage >= 0 {
                candidate_valid = false;
                break;
            }
            let Some(raw_damage) = signed_damage.checked_neg().map(|value| value as u32) else {
                candidate_valid = false;
                break;
            };
            if !current_hp.is_finite()
                || !(0.0..=MAX_COMBAT_VALUE).contains(&current_hp)
                || dead_state > 1
                || wrapper_count != 2
                || first_type[0] != SERVER_DAMAGE_WRAPPER_TYPE
                || first_length != 4
                || second_type[0] != SERVER_DAMAGE_SECONDARY_WRAPPER_TYPE
                || second_length != 4
                || (raw_damage as f32) > MAX_COMBAT_VALUE
            {
                candidate_valid = false;
                break;
            }
            if (raw_damage as f32) < MIN_DAMAGE {
                continue;
            }
            let damage_bit_offset =
                element_bit_offset + CLIENT_FIGHT_DATA_FIRST_WRAPPER_VALUE_BIT_OFFSET;
            candidate.push(ParsedServerDamageSettlement {
                target_handle,
                current_hp,
                dead_state,
                raw_damage,
                secondary_value: i32::from_le_bytes(second_value_bytes),
                byte_offset: damage_bit_offset / 8,
                bit_shift: (damage_bit_offset % 8) as u8,
            });
        }
        if candidate_valid {
            for settlement in candidate {
                if seen.insert((
                    settlement.byte_offset,
                    settlement.bit_shift,
                    settlement.target_handle,
                    settlement.current_hp.to_bits(),
                    settlement.raw_damage,
                )) {
                    settlements.push(settlement);
                }
            }
        }
    }
    settlements
}

/// Decodes the validated `ClientSetReplicatedTargetData + ClientDamageBoss` wire layout.
///
/// The shape and field position were independently stable in both chronological halves of the
/// reference capture. Every structural condition remains mandatory so unrelated packets fall
/// through to the generic bounded scanner.
pub fn parse_client_damage_boss_update(
    packet: &SequencedPacket,
    bunch_packet: &BunchPacket,
) -> Option<ParsedBossHpUpdate> {
    let [located] = bunch_packet.bunches.as_slice() else {
        return None;
    };
    if packet.mode != 0
        || packet.payload_bit_len != CLIENT_DAMAGE_BOSS_PAYLOAD_BITS
        || bunch_packet.packet_info_bit_len != CLIENT_DAMAGE_BOSS_PACKET_INFO_BITS
        || located.bit_offset != CLIENT_DAMAGE_BOSS_PACKET_INFO_BITS
        || located.bunch.descriptor != 0
        || located.bunch.partial_flags != 0x0c
        || located.bunch.data_bit_len != 91
    {
        return None;
    }
    parse_boss_hp_update_at(
        &packet.payload,
        CLIENT_DAMAGE_BOSS_PREFIX_BYTE_OFFSET,
        CLIENT_DAMAGE_BOSS_BIT_SHIFT,
        None,
    )
}

pub fn parse_boss_hp_updates(data: &[u8]) -> Vec<ParsedBossHpUpdate> {
    let mut updates = parse_client_fight_target_updates(data);
    let mut locations = updates
        .iter()
        .map(|update| (update.byte_offset, update.bit_shift))
        .collect::<HashSet<_>>();
    let mut packet_handles = HashSet::new();
    packet_handles.extend(updates.iter().map(|update| update.target_handle));
    for byte_offset in 0..data.len() {
        for bit_shift in 0..8_u8 {
            if let Some(update) =
                parse_boss_hp_update_at(data, byte_offset, bit_shift, Some(&packet_handles))
                && locations.insert((update.byte_offset, update.bit_shift))
            {
                packet_handles.insert(update.target_handle);
                updates.push(update);
            }
        }
    }
    updates
}

pub fn parse_gameplay_effects(data: &[u8]) -> Vec<ParsedGameplayEffect> {
    let mut effects = Vec::new();
    let mut seen = HashSet::new();
    for bit_shift in 0..8_u8 {
        let shifted = if bit_shift == 0 {
            data.to_vec()
        } else {
            match decode_shifted_bytes(data, 0, bit_shift, 0, data.len().saturating_sub(1)) {
                Some(value) => value,
                None => continue,
            }
        };
        for (anchor_offset, window) in shifted
            .windows(ACTIVE_GAMEPLAY_EFFECT_ANCHOR.len())
            .enumerate()
        {
            if window != ACTIVE_GAMEPLAY_EFFECT_ANCHOR {
                continue;
            }
            let marker_offset = anchor_offset
                + ACTIVE_GAMEPLAY_EFFECT_ANCHOR.len()
                + ACTIVE_GAMEPLAY_EFFECT_VALUE_OFFSET;
            let Some(marker_bytes) = shifted.get(marker_offset..marker_offset + 4) else {
                continue;
            };
            let marker = u32::from_le_bytes(marker_bytes.try_into().unwrap());
            if marker != ACTIVE_GAMEPLAY_EFFECT_MARKER {
                continue;
            }
            let index_offset = marker_offset + 4;
            let Some(index_bytes) = shifted.get(index_offset..index_offset + 4) else {
                continue;
            };
            let unique_index = u32::from_le_bytes(index_bytes.try_into().unwrap());
            if matches!(unique_index, 0 | u32::MAX)
                || !seen.insert((unique_index, bit_shift, index_offset))
            {
                continue;
            }
            effects.push(ParsedGameplayEffect {
                unique_index,
                byte_offset: index_offset,
                bit_shift,
            });
        }
        for marker_offset in 0..shifted.len() {
            if shifted
                .get(
                    marker_offset + 4
                        ..marker_offset + 4 + BOOL_ENUM_COMPACT_GAMEPLAY_EFFECT_TRAILER.len(),
                )
                .is_some_and(|bytes| bytes == BOOL_ENUM_COMPACT_GAMEPLAY_EFFECT_TRAILER)
                && let Some(index_bytes) = shifted.get(marker_offset..marker_offset + 4)
                && let Ok(index_bytes) = <[u8; 4]>::try_from(index_bytes)
            {
                let unique_index = u32::from_le_bytes(index_bytes);
                if !matches!(unique_index, 0 | u32::MAX)
                    && seen.insert((unique_index, bit_shift, marker_offset))
                {
                    effects.push(ParsedGameplayEffect {
                        unique_index,
                        byte_offset: marker_offset,
                        bit_shift,
                    });
                }
            }
            let trailer_offset = match shifted[marker_offset] {
                COMPACT_GAMEPLAY_EFFECT_MARKER_WITH_FIELD
                    if shifted
                        .get(marker_offset + 5..marker_offset + 9)
                        .is_some_and(|bytes| bytes == COMPACT_GAMEPLAY_EFFECT_FIELD) =>
                {
                    marker_offset + 13
                }
                COMPACT_GAMEPLAY_EFFECT_MARKER => marker_offset + 9,
                _ => continue,
            };
            if shifted
                .get(trailer_offset..trailer_offset + COMPACT_GAMEPLAY_EFFECT_TRAILER.len())
                .is_none_or(|bytes| bytes != COMPACT_GAMEPLAY_EFFECT_TRAILER)
            {
                continue;
            }
            let index_offset = marker_offset + 1;
            let unique_index = u32::from_le_bytes(
                shifted[index_offset..index_offset + 4]
                    .try_into()
                    .expect("Compact GameplayEffect index has a fixed four-byte length"),
            );
            if matches!(unique_index, 0 | u32::MAX)
                || !seen.insert((unique_index, bit_shift, index_offset))
            {
                continue;
            }
            effects.push(ParsedGameplayEffect {
                unique_index,
                byte_offset: index_offset,
                bit_shift,
            });
        }
    }
    effects
}

pub fn matches_shifted_bytes_at(
    data: &[u8],
    bit_shift: u8,
    byte_offset: usize,
    expected: &[u8],
) -> bool {
    let Some(decoded) = decode_shifted_bytes(data, 0, bit_shift, 0, data.len().saturating_sub(1))
    else {
        return false;
    };
    decoded
        .get(byte_offset..byte_offset + expected.len())
        .is_some_and(|bytes| bytes == expected)
}

pub fn find_declared_character_evidence(data: &[u8]) -> Vec<(u32, u8, usize)> {
    let mut found = Vec::new();
    for bit_shift in 0..8 {
        let shifted = if bit_shift == 0 {
            data.to_vec()
        } else {
            match decode_shifted_bytes(data, 0, bit_shift, 0, data.len().saturating_sub(1)) {
                Some(value) => value,
                None => continue,
            }
        };
        if shifted.len() < 9 {
            continue;
        }
        for offset in 0..=shifted.len() - 9 {
            let row = &shifted[offset..offset + 9];
            if row[..4] != [5, 0, 0, 0] || row[8] != 0 {
                continue;
            }
            if row[4..8].iter().all(u8::is_ascii_digit) {
                let id = row[4..8]
                    .iter()
                    .fold(0_u32, |value, digit| value * 10 + (digit - b'0') as u32);
                let evidence = (id, bit_shift, offset);
                if (1000..=9999).contains(&id) && !found.contains(&evidence) {
                    found.push(evidence);
                }
            }
        }
    }
    found
}

pub fn find_final_tower_character_evidence(data: &[u8]) -> Vec<(u32, u8, usize)> {
    const CHARACTER_FOR_NET: &[u8] = b"FCharacterForNet";
    const FINAL_TOWER_CHARACTER: &[u8] = b"ft_character_";

    let mut found = Vec::new();
    for bit_shift in 0..8 {
        let shifted = if bit_shift == 0 {
            data.to_vec()
        } else {
            match decode_shifted_bytes(data, 0, bit_shift, 0, data.len().saturating_sub(1)) {
                Some(value) => value,
                None => continue,
            }
        };
        if shifted.len() < CHARACTER_FOR_NET.len() + FINAL_TOWER_CHARACTER.len() + 4 {
            continue;
        }
        if !shifted
            .windows(CHARACTER_FOR_NET.len())
            .any(|window| window == CHARACTER_FOR_NET)
        {
            continue;
        }
        for offset in 0..=shifted.len() - FINAL_TOWER_CHARACTER.len() - 4 {
            if &shifted[offset..offset + FINAL_TOWER_CHARACTER.len()] != FINAL_TOWER_CHARACTER {
                continue;
            }
            let digit_offset = offset + FINAL_TOWER_CHARACTER.len();
            let digits = &shifted[digit_offset..digit_offset + 4];
            if !digits.iter().all(u8::is_ascii_digit) {
                continue;
            }
            if shifted
                .get(digit_offset + 4)
                .is_some_and(u8::is_ascii_digit)
            {
                continue;
            }
            let id = digits
                .iter()
                .fold(0_u32, |value, digit| value * 10 + (digit - b'0') as u32);
            let evidence = (id, bit_shift, offset);
            if (1000..=9999).contains(&id) && !found.contains(&evidence) {
                found.push(evidence);
            }
        }
    }
    found
}

pub fn declared_character_ids_from_evidence(evidence: &[(u32, u8, usize)]) -> Vec<u32> {
    let mut ids = Vec::new();
    for (id, _, _) in evidence {
        if !ids.contains(id) {
            ids.push(*id);
        }
    }
    ids
}

fn declared_character_for_shift(evidence: &[(u32, u8, usize)], bit_shift: u8) -> Option<u32> {
    let mut matched = None;
    for (id, shift, _) in evidence {
        if *shift != bit_shift {
            continue;
        }
        if matched.is_some_and(|current| current != *id) {
            return None;
        }
        matched = Some(*id);
    }
    matched
}

fn damage_record_targets_declared_character(
    record: &ParsedDamageRecord,
    character_id: Option<u32>,
    evidence: &[(u32, u8, usize)],
) -> bool {
    let Some(character_id) = character_id else {
        return false;
    };
    evidence.iter().any(|(id, shift, offset)| {
        *id == character_id && *shift == record.bit_shift && *offset > record.byte_offset
    })
}

pub fn parse_damage_payload(
    data: &[u8],
    timestamp: f64,
    packet_char_id: Option<u32>,
    fallback_char_id: Option<u32>,
    characters: &HashMap<u32, CharacterInfo>,
    evidence: &[(u32, u8, usize)],
) -> Vec<Hit> {
    let mut hits = Vec::new();
    for record in parse_damage_records(data) {
        let damage = record.damage;
        let byte_offset = record.byte_offset;
        let bit_shift = record.bit_shift;
        let aligned_char_id = declared_character_for_shift(evidence, bit_shift);
        let resolved_packet_char_id = packet_char_id.or(aligned_char_id);
        let char_id = resolved_packet_char_id.or(fallback_char_id).unwrap_or(0);
        let target_hp_before = record.target_hp_before;
        let target_max_hp = record.target_max_hp;
        let character = characters.get(&char_id);
        let name = character
            .map(|row| {
                if row.name_zh.is_empty() {
                    row.name_en.clone()
                } else {
                    row.name_zh.clone()
                }
            })
            .unwrap_or_else(|| {
                if char_id == 0 {
                    "未知角色".to_owned()
                } else {
                    format!("未知角色({char_id})")
                }
            });
        let target_hp_after = (target_hp_before - damage).max(0.0);
        let targets_declared_character =
            damage_record_targets_declared_character(&record, resolved_packet_char_id, evidence);
        let direction = if targets_declared_character {
            HitDirection::Incoming
        } else if resolved_packet_char_id.is_some() {
            HitDirection::Outgoing
        } else {
            HitDirection::Unknown
        };
        let record_bit_offset = record
            .byte_offset
            .checked_mul(8)
            .and_then(|bit_offset| bit_offset.checked_add(usize::from(record.bit_shift)));
        let target_handle = record_bit_offset
            .and_then(|bit_offset| {
                bit_offset.checked_add(BITPACKED_DAMAGE_RECORD_TARGET_WIRE_BIT_DELTA)
            })
            .and_then(|bit_offset| parse_client_fight_target_wire_identity(data, bit_offset))
            .or_else(|| {
                record_bit_offset
                    .and_then(|bit_offset| {
                        bit_offset.checked_add(COMPACT_DAMAGE_RECORD_TARGET_WIRE_BIT_DELTA)
                    })
                    .and_then(|bit_offset| {
                        parse_compact_client_fight_target_identity(data, bit_offset)
                    })
            });
        let (target_id, target_context) = match target_handle {
            Some(target_handle) => {
                let wire_id = hex::encode(target_handle);
                (
                    Some(format!("enemy-wire:{wire_id}")),
                    vec![format!("enemy_target_wire={wire_id}")],
                )
            }
            None => (None, Vec::new()),
        };
        let hit = Hit {
            timestamp,
            char_id,
            char_name: name,
            char_known: character.is_some(),
            damage: damage as f64,
            byte_offset,
            bit_shift,
            char_source: if resolved_packet_char_id.is_some() {
                HitCharacterSource::Packet
            } else if fallback_char_id.is_some() {
                HitCharacterSource::Session
            } else {
                HitCharacterSource::Unknown
            },
            direction,
            target_hp_before: target_hp_before as f64,
            target_hp_after: target_hp_after as f64,
            target_max_hp: target_max_hp as f64,
            target_hp_percent: if target_max_hp > 0.0 {
                target_hp_after as f64 / target_max_hp as f64 * 100.0
            } else {
                0.0
            },
            target_id,
            target_name: None,
            target_name_en: None,
            target_name_ja: None,
            target_monster_id: None,
            target_context,
            gameplay_effect_index: None,
            gameplay_effect_name: None,
            ability_name: None,
            damage_name: None,
            damage_component: None,
            attack_type: None,
            damage_attribute: None,
            follow_up_damage: 0.0,
            follow_up_timestamp: None,
            follow_up_damage_name: None,
            follow_up_attack_type: None,
            follow_up_damage_attribute: None,
            reconciled_overkill_damage: None,
            wire_event: Some(DamageWireEvent {
                damage: record.damage,
                target_hp_before: record.target_hp_before,
                target_max_hp: record.target_max_hp,
                damage_time: record.damage_time,
                world_time: record.world_time,
                repeated_damage: record.repeated_damage,
                state_flags: record.state_flags,
                trailing_value: record.trailing_value,
            }),
        };
        hits.push(hit);
    }
    hits
}

#[cfg(test)]
mod character_tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEMP_JSON_COUNTER: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn enemy_config_hash_is_stable_and_case_insensitive() {
        assert_eq!(
            enemy_config_hash("Boss_016_BP"),
            Some(0x4d88_7b49_05d5_dbaf)
        );
        assert_eq!(
            enemy_config_hash("BOSS_016_BP"),
            enemy_config_hash("boss_016_bp")
        );
        assert_eq!(enemy_config_hash(""), None);
        assert_eq!(enemy_config_hash("敌人"), None);
    }

    #[test]
    fn bundled_enemy_catalog_resolves_official_names_and_portrait() {
        let catalog =
            load_enemy_catalog(Path::new(ENEMY_CATALOG_PATH)).expect("enemy catalog should load");
        let identity = catalog
            .get(enemy_config_hash("Boss_016_BP").expect("ASCII config ID"))
            .expect("known enemy config should resolve");

        assert_eq!(identity.config_id, "Boss_016_BP");
        assert_eq!(identity.monster_id, "Boss_16");
        assert_eq!(identity.name_en, "Imaginadough");
        assert_eq!(identity.name_zh, "随心泥");
        assert_eq!(identity.name_ja, "イメージクレイ");
    }

    fn encoded_damage_record_with_flags(
        damage: f32,
        target_hp_before: f32,
        target_max_hp: f32,
        state_flags: [i32; 3],
    ) -> Vec<u8> {
        let fields = [
            (12, damage.to_le_bytes().to_vec()),
            (12, target_hp_before.to_le_bytes().to_vec()),
            (12, target_max_hp.to_le_bytes().to_vec()),
            (13, 1.0_f64.to_le_bytes().to_vec()),
            (12, 1.0_f32.to_le_bytes().to_vec()),
            (12, damage.to_le_bytes().to_vec()),
            (6, state_flags[0].to_le_bytes().to_vec()),
            (6, state_flags[1].to_le_bytes().to_vec()),
            (6, state_flags[2].to_le_bytes().to_vec()),
            (12, 0.0_f32.to_le_bytes().to_vec()),
        ];
        let mut encoded = Vec::new();
        for (field_type, value) in fields {
            encoded.push(field_type);
            encoded.extend_from_slice(&(value.len() as u32).to_le_bytes());
            encoded.extend_from_slice(&value);
        }
        encoded
    }

    fn encoded_damage_record(damage: f32, target_hp_before: f32, target_max_hp: f32) -> Vec<u8> {
        encoded_damage_record_with_flags(damage, target_hp_before, target_max_hp, [0, 0, 0])
    }

    fn encoded_bool_enum_damage_record_with_flags(
        damage: f32,
        target_hp_before: f32,
        target_max_hp: f32,
        state_flags: [i32; 3],
    ) -> Vec<u8> {
        let fields = [
            (12, damage.to_le_bytes().to_vec()),
            (12, target_hp_before.to_le_bytes().to_vec()),
            (12, target_max_hp.to_le_bytes().to_vec()),
            (13, 1.0_f64.to_le_bytes().to_vec()),
            (12, 1.0_f32.to_le_bytes().to_vec()),
            (12, damage.to_le_bytes().to_vec()),
            (8, vec![state_flags[0] as u8]),
            (1, state_flags[1].to_le_bytes().to_vec()),
            (1, state_flags[2].to_le_bytes().to_vec()),
            (12, 0.0_f32.to_le_bytes().to_vec()),
        ];
        let mut encoded = Vec::new();
        for (field_type, value) in fields {
            encoded.push(field_type);
            encoded.extend_from_slice(&(value.len() as u32).to_le_bytes());
            encoded.extend_from_slice(&value);
        }
        encoded
    }

    fn write_shifted_bytes(payload: &mut [u8], bit_shift: u8, byte_offset: usize, bytes: &[u8]) {
        for (index, byte) in bytes.iter().enumerate() {
            for bit in 0..8 {
                let bit_value = (byte >> bit) & 1;
                let target_bit = bit_shift as usize + (byte_offset + index) * 8 + bit;
                let target_byte = target_bit / 8;
                let target_bit_offset = target_bit % 8;
                if bit_value == 1 {
                    payload[target_byte] |= 1 << target_bit_offset;
                } else {
                    payload[target_byte] &= !(1 << target_bit_offset);
                }
            }
        }
    }

    fn write_bytes_at_bit(payload: &mut [u8], bit_offset: usize, bytes: &[u8]) {
        write_shifted_bytes(payload, (bit_offset % 8) as u8, bit_offset / 8, bytes);
    }

    fn client_fight_target_identity(discriminator: u8) -> [u8; CLIENT_FIGHT_TARGET_WIRE_BYTES] {
        let mut identity = [0; CLIENT_FIGHT_TARGET_WIRE_BYTES];
        identity[..16].copy_from_slice(&[
            0x08, 0x81, 0x77, 0x92, 0x4c, 0xac, 0x9c, 0x79, 0x62, 0x5c, 0xe0, 0x55, 0x75, 0x08,
            0x7f, 0x3b,
        ]);
        identity[16] = 0x0f;
        identity[20] = discriminator;
        identity
    }

    fn write_client_fight_array(
        payload: &mut [u8],
        record_bit_offset: usize,
        terminal_death: bool,
        elements: &[([u8; CLIENT_FIGHT_TARGET_WIRE_BYTES], f32, u32)],
    ) {
        let mut prefix = [0; BOSS_HP_PREFIX_HEAD.len()];
        if !terminal_death {
            prefix[0] = BOSS_HP_PREFIX_HEAD[0];
        }
        prefix[5..7].copy_from_slice(&((elements.len() as u16) << 5).to_le_bytes());
        write_bytes_at_bit(payload, record_bit_offset, &prefix);
        for (index, (identity, hp, dead_state)) in elements.iter().enumerate() {
            let element_bit_offset = record_bit_offset + index * CLIENT_FIGHT_DATA_ELEMENT_BITS;
            write_bytes_at_bit(
                payload,
                element_bit_offset + CLIENT_FIGHT_DATA_ARRAY_HEADER_BITS,
                identity,
            );
            write_bytes_at_bit(
                payload,
                element_bit_offset + CLIENT_FIGHT_DATA_HP_BIT_OFFSET,
                &hp.to_le_bytes(),
            );
            write_bytes_at_bit(
                payload,
                element_bit_offset + CLIENT_FIGHT_DATA_DEAD_STATE_BIT_OFFSET,
                &dead_state.to_le_bytes(),
            );
        }
    }

    fn write_client_fight_damage_array(
        payload: &mut [u8],
        record_bit_offset: usize,
        elements: &[([u8; CLIENT_FIGHT_TARGET_WIRE_BYTES], f32, u32, u32, i32)],
    ) {
        let base_elements = elements
            .iter()
            .map(|(identity, hp, dead_state, _, _)| (*identity, *hp, *dead_state))
            .collect::<Vec<_>>();
        write_client_fight_array(payload, record_bit_offset, false, &base_elements);
        for (index, (_, _, _, raw_damage, secondary_value)) in elements.iter().enumerate() {
            let element_bit_offset = record_bit_offset + index * CLIENT_FIGHT_DATA_ELEMENT_BITS;
            write_bytes_at_bit(
                payload,
                element_bit_offset + CLIENT_FIGHT_DATA_WRAPPER_COUNT_BIT_OFFSET,
                &2_u32.to_le_bytes(),
            );
            write_bytes_at_bit(
                payload,
                element_bit_offset + CLIENT_FIGHT_DATA_FIRST_WRAPPER_TYPE_BIT_OFFSET,
                &[SERVER_DAMAGE_WRAPPER_TYPE],
            );
            write_bytes_at_bit(
                payload,
                element_bit_offset + CLIENT_FIGHT_DATA_FIRST_WRAPPER_LENGTH_BIT_OFFSET,
                &4_u32.to_le_bytes(),
            );
            write_bytes_at_bit(
                payload,
                element_bit_offset + CLIENT_FIGHT_DATA_FIRST_WRAPPER_VALUE_BIT_OFFSET,
                &(-(*raw_damage as i32)).to_le_bytes(),
            );
            write_bytes_at_bit(
                payload,
                element_bit_offset + CLIENT_FIGHT_DATA_SECOND_WRAPPER_TYPE_BIT_OFFSET,
                &[SERVER_DAMAGE_SECONDARY_WRAPPER_TYPE],
            );
            write_bytes_at_bit(
                payload,
                element_bit_offset + CLIENT_FIGHT_DATA_SECOND_WRAPPER_LENGTH_BIT_OFFSET,
                &4_u32.to_le_bytes(),
            );
            write_bytes_at_bit(
                payload,
                element_bit_offset + CLIENT_FIGHT_DATA_SECOND_WRAPPER_VALUE_BIT_OFFSET,
                &secondary_value.to_le_bytes(),
            );
        }
    }

    fn push_fstring(buffer: &mut Vec<u8>, value: &str) {
        buffer.extend_from_slice(&((value.len() + 1) as i32).to_le_bytes());
        buffer.extend_from_slice(value.as_bytes());
        buffer.push(0);
    }

    fn push_property_header(buffer: &mut Vec<u8>, name: &str, property_type: &str, size: i32) {
        push_fstring(buffer, name);
        push_fstring(buffer, property_type);
        buffer.extend_from_slice(&0_i32.to_le_bytes());
        buffer.extend_from_slice(&size.to_le_bytes());
    }

    fn push_i32_property(buffer: &mut Vec<u8>, name: &str, value: i32) {
        push_property_header(buffer, name, "IntProperty", 4);
        buffer.push(0);
        buffer.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32_property(buffer: &mut Vec<u8>, name: &str, value: u32) {
        push_property_header(buffer, name, "UInt32Property", 4);
        buffer.push(0);
        buffer.extend_from_slice(&value.to_le_bytes());
    }

    fn push_name_property(buffer: &mut Vec<u8>, name: &str, value: &str) {
        push_property_header(buffer, name, "NameProperty", (value.len() + 5) as i32);
        buffer.push(0);
        push_fstring(buffer, value);
    }

    fn push_bool_property(buffer: &mut Vec<u8>, name: &str, value: u8) {
        push_property_header(buffer, name, "BoolProperty", 0);
        buffer.push(value);
    }

    fn push_none_property(buffer: &mut Vec<u8>) {
        push_fstring(buffer, "None");
    }

    fn push_ht_item_net_id_property(buffer: &mut Vec<u8>, solt: u32, serial: u32) {
        let mut nested = Vec::new();
        push_u32_property(&mut nested, "solt", solt);
        push_u32_property(&mut nested, "serial", serial);
        push_none_property(&mut nested);

        push_fstring(buffer, "EquipNetID");
        push_fstring(buffer, "StructProperty");
        buffer.extend_from_slice(&1_i32.to_le_bytes());
        push_fstring(buffer, "HTItemNetID");
        buffer.extend_from_slice(&1_i32.to_le_bytes());
        push_fstring(buffer, "/Script/HTGame");
        buffer.extend_from_slice(&0_i32.to_le_bytes());
        buffer.extend_from_slice(&(nested.len() as i32).to_le_bytes());
        buffer.push(0);
        buffer.extend_from_slice(&nested);
    }

    fn encoded_equipment_cell(
        state: i32,
        equipment_id: &str,
        solt: u32,
        serial: u32,
        first_step: bool,
        row: i32,
        column: i32,
    ) -> Vec<u8> {
        let mut payload = Vec::new();
        push_i32_property(&mut payload, "State", state);
        push_name_property(&mut payload, "EquipmentID", equipment_id);
        push_ht_item_net_id_property(&mut payload, solt, serial);
        push_bool_property(&mut payload, "bFirstStep", u8::from(first_step));
        push_i32_property(&mut payload, "Row", row);
        push_i32_property(&mut payload, "Column", column);
        push_i32_property(&mut payload, "New", 0);
        push_none_property(&mut payload);
        payload
    }

    fn encoded_equipment_slot(
        equipment_id: &str,
        solt: u32,
        serial: u32,
        row: i32,
        column: i32,
    ) -> Vec<u8> {
        encoded_equipment_cell(1, equipment_id, solt, serial, true, row, column)
    }

    fn empty_curtain_test_item(id: HtItemNetId, item_id: &str) -> EmptyCurtainItem {
        EmptyCurtainItem {
            id,
            item_id: item_id.to_owned(),
            level: 0,
            main_stats: Vec::new(),
            sub_stats: Vec::new(),
            locked: false,
            discarded: false,
            character_net_id: None,
            equipped_character_id: None,
            equipped_placement: None,
        }
    }

    fn empty_curtain_test_catalog() -> EquipmentCatalog {
        load_equipment_catalog(Path::new(EQUIPMENT_CATALOG_PATH))
            .expect("bundled equipment catalog should load")
    }

    fn write_temp_json(name: &str, content: &str) -> PathBuf {
        let unique = TEMP_JSON_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "nte_dps_tool_{}_{}_{}",
            std::process::id(),
            unique,
            name
        ));
        fs::write(&path, content).expect("temp json should be writable");
        path
    }

    #[test]
    fn parses_shifted_equipment_slot_info() {
        let encoded =
            encoded_equipment_slot("cell3_style5_1_Orange", 1_018_562_417, 1_290_515_095, 2, 3);
        let mut payload = vec![0; encoded.len() + 2];
        write_shifted_bytes(&mut payload, 5, 0, &encoded);

        assert_eq!(
            parse_equipment_slots(&payload),
            vec![ParsedEquipmentSlot {
                state: 1,
                equipment_id: "cell3_style5_1_Orange".to_owned(),
                equip_net_id: ParsedHtItemNetId {
                    solt: 1_018_562_417,
                    serial: 1_290_515_095,
                },
                first_step: true,
                row: 2,
                column: 3,
                new_flag: 0,
                byte_offset: 0,
                bit_shift: 5,
            }]
        );
    }

    #[test]
    fn ignores_incomplete_equipment_slot_tags() {
        let mut payload = Vec::new();
        push_i32_property(&mut payload, "State", 1);
        push_name_property(&mut payload, "EquipmentID", "cell2_style1_1_Orange");

        assert!(parse_equipment_slots(&payload).is_empty());
    }

    #[test]
    fn parses_complete_character_module_grid_snapshot() {
        let character_net_id = HtItemNetId {
            solt: 300,
            serial: 400,
        };
        let module_id = HtItemNetId {
            solt: 500,
            serial: 600,
        };
        let mut payload = Vec::new();
        for row in 0..EMPTY_CURTAIN_GRID_SIDE {
            for column in 0..EMPTY_CURTAIN_GRID_SIDE {
                let first_step = (row, column) == (1, 2);
                let occupied = first_step || matches!((row, column), (2, 1) | (2, 2));
                payload.extend_from_slice(&if occupied {
                    encoded_equipment_cell(
                        1,
                        "cell3_style6_1_Orange",
                        module_id.solt,
                        module_id.serial,
                        first_step,
                        row,
                        column,
                    )
                } else {
                    encoded_equipment_cell(0, "None", 0, 0, false, row, column)
                });
            }
        }
        let characters = HashMap::from([(character_net_id, 1023)]);
        let known_items = HashMap::from([(
            module_id,
            empty_curtain_test_item(module_id, "cell3_style6_1_Orange"),
        )]);

        let expected = Some(ParsedEmptyCurtainEquipmentSnapshot::Modules {
            character_net_id,
            character_id: 1023,
            placements: vec![ParsedEmptyCurtainModulePlacement {
                equipment: module_id,
                item_id: "cell3_style6_1_Orange".to_owned(),
                row: 1,
                column: 2,
            }],
        });
        let catalog = empty_curtain_test_catalog();
        assert_eq!(
            parse_empty_curtain_equipment_snapshot(
                &payload,
                payload.len() * 8,
                &characters,
                &known_items,
                &catalog,
            ),
            expected.clone()
        );
        assert_eq!(
            parse_empty_curtain_equipment_snapshot(
                &payload,
                payload.len() * 8,
                &characters,
                &HashMap::new(),
                &catalog,
            ),
            expected
        );

        let mismatched_items = HashMap::from([(
            module_id,
            empty_curtain_test_item(module_id, "cell3_style1_1_Orange"),
        )]);
        assert_eq!(
            parse_empty_curtain_equipment_snapshot(
                &payload,
                payload.len() * 8,
                &characters,
                &mismatched_items,
                &catalog,
            ),
            None
        );

        let mut wrong_geometry = Vec::new();
        for row in 0..EMPTY_CURTAIN_GRID_SIDE {
            for column in 0..EMPTY_CURTAIN_GRID_SIDE {
                let first_step = (row, column) == (1, 2);
                let occupied = first_step || matches!((row, column), (1, 3) | (1, 4));
                wrong_geometry.extend_from_slice(&if occupied {
                    encoded_equipment_cell(
                        1,
                        "cell3_style6_1_Orange",
                        module_id.solt,
                        module_id.serial,
                        first_step,
                        row,
                        column,
                    )
                } else {
                    encoded_equipment_cell(0, "None", 0, 0, false, row, column)
                });
            }
        }
        assert_eq!(
            parse_empty_curtain_equipment_snapshot(
                &wrong_geometry,
                wrong_geometry.len() * 8,
                &characters,
                &known_items,
                &empty_curtain_test_catalog(),
            ),
            None
        );
    }

    #[test]
    fn rejects_partial_or_inconsistent_character_module_grid() {
        let character_net_id = HtItemNetId {
            solt: 300,
            serial: 400,
        };
        let module_id = HtItemNetId {
            solt: 500,
            serial: 600,
        };
        let characters = HashMap::from([(character_net_id, 1023)]);
        let known_items = HashMap::from([(
            module_id,
            empty_curtain_test_item(module_id, "cell3_style6_1_Orange"),
        )]);
        let payload = encoded_equipment_cell(
            1,
            "cell3_style6_1_Orange",
            module_id.solt,
            module_id.serial,
            true,
            1,
            2,
        );

        assert_eq!(
            parse_empty_curtain_equipment_snapshot(
                &payload,
                payload.len() * 8,
                &characters,
                &known_items,
                &empty_curtain_test_catalog(),
            ),
            None
        );
    }

    #[test]
    fn parses_equipped_and_empty_character_core_snapshots() {
        let character_net_id = HtItemNetId {
            solt: 300,
            serial: 400,
        };
        let core_id = HtItemNetId {
            solt: 700,
            serial: 800,
        };
        let characters = HashMap::from([(character_net_id, 1023)]);
        let known_items = HashMap::from([(
            core_id,
            empty_curtain_test_item(core_id, "Incantation_orange"),
        )]);
        let mut encoded = EMPTY_CURTAIN_CORE_SLOT_ANCHOR.to_vec();
        encoded.extend_from_slice(&core_id.solt.to_le_bytes());
        encoded.extend_from_slice(&core_id.serial.to_le_bytes());
        let mut equipped = vec![0; encoded.len() + 1];
        write_shifted_bytes(&mut equipped, 3, 0, &encoded);

        assert_eq!(
            parse_empty_curtain_equipment_snapshot(
                &equipped,
                3 + encoded.len() * 8,
                &characters,
                &known_items,
                &empty_curtain_test_catalog(),
            ),
            Some(ParsedEmptyCurtainEquipmentSnapshot::Core {
                character_net_id,
                character_id: 1023,
                item_id: Some(core_id),
            })
        );
        assert_eq!(
            parse_empty_curtain_equipment_snapshot(
                EMPTY_CURTAIN_CORE_SLOT_ANCHOR,
                EMPTY_CURTAIN_CORE_SLOT_ANCHOR.len() * 8,
                &characters,
                &known_items,
                &empty_curtain_test_catalog(),
            ),
            Some(ParsedEmptyCurtainEquipmentSnapshot::Core {
                character_net_id,
                character_id: 1023,
                item_id: None,
            })
        );
    }

    #[test]
    fn core_snapshot_requires_one_character_and_at_most_one_known_core() {
        let first_character = HtItemNetId {
            solt: 300,
            serial: 400,
        };
        let second_character = HtItemNetId {
            solt: 301,
            serial: 401,
        };
        let first_core = HtItemNetId {
            solt: 700,
            serial: 800,
        };
        let second_core = HtItemNetId {
            solt: 701,
            serial: 801,
        };
        let mut payload = EMPTY_CURTAIN_CORE_SLOT_ANCHOR.to_vec();
        for id in [first_core, second_core] {
            payload.extend_from_slice(&id.solt.to_le_bytes());
            payload.extend_from_slice(&id.serial.to_le_bytes());
        }
        let known_items = HashMap::from([
            (
                first_core,
                empty_curtain_test_item(first_core, "Incantation_orange"),
            ),
            (
                second_core,
                empty_curtain_test_item(second_core, "Incantation_orange"),
            ),
        ]);

        assert_eq!(
            parse_empty_curtain_equipment_snapshot(
                &payload,
                payload.len() * 8,
                &HashMap::from([(first_character, 1023)]),
                &known_items,
                &empty_curtain_test_catalog(),
            ),
            None
        );
        assert_eq!(
            parse_empty_curtain_equipment_snapshot(
                EMPTY_CURTAIN_CORE_SLOT_ANCHOR,
                EMPTY_CURTAIN_CORE_SLOT_ANCHOR.len() * 8,
                &HashMap::from([(first_character, 1023), (second_character, 1020)]),
                &known_items,
                &empty_curtain_test_catalog(),
            ),
            None
        );
    }

    #[test]
    fn character_attribute_is_optional_and_loaded_when_present() {
        let document: CharacterDocument = serde_json::from_str(
            r#"{
                "characters": {
                    "1003": {"name_zh": "Sagiri"},
                    "1010": {"name_zh": "Nanally", "attribute": "curse"}
                }
            }"#,
        )
        .unwrap();

        assert_eq!(document.characters["1003"].attribute, None);
        assert_eq!(
            document.characters["1010"].attribute.as_deref(),
            Some("curse")
        );
    }

    #[test]
    #[cfg(not(feature = "external_resources"))]
    fn load_characters_falls_back_to_bundled_resource() {
        let characters = load_characters(Path::new(
            "missing-root/res/data/characters/characters.json",
        ))
        .expect("bundled characters should load");

        let canhong = characters.get(&1036).expect("残虹 should be bundled");
        assert_eq!(canhong.name_zh, "残虹");
        assert_eq!(canhong.attribute.as_deref(), Some("咒"));
        assert!(
            canhong
                .avatar
                .as_deref()
                .is_some_and(|path| !path.is_empty())
        );

        let lingke = characters.get(&1072).expect("灵可 should be bundled");
        assert_eq!(lingke.name_zh, "灵可");
        assert_eq!(lingke.attribute.as_deref(), Some("灵"));
        assert!(
            lingke
                .avatar
                .as_deref()
                .is_some_and(|path| !path.is_empty())
        );

        assert!(
            !characters.contains_key(&1091),
            "non-canonical alias row must not become a third character"
        );
    }

    #[test]
    fn parses_gameplay_effect_unique_index_after_active_ge_anchor() {
        let mut payload = vec![0xaa, 0xbb];
        payload.extend_from_slice(ACTIVE_GAMEPLAY_EFFECT_ANCHOR);
        payload.extend_from_slice(&[0; ACTIVE_GAMEPLAY_EFFECT_VALUE_OFFSET]);
        payload.extend_from_slice(&ACTIVE_GAMEPLAY_EFFECT_MARKER.to_le_bytes());
        payload.extend_from_slice(&1012_u32.to_le_bytes());
        payload.extend_from_slice(&[5, 0, 0, 0]);

        assert_eq!(
            parse_gameplay_effects(&payload),
            vec![ParsedGameplayEffect {
                unique_index: 1012,
                byte_offset: 2
                    + ACTIVE_GAMEPLAY_EFFECT_ANCHOR.len()
                    + ACTIVE_GAMEPLAY_EFFECT_VALUE_OFFSET
                    + 4,
                bit_shift: 0,
            }]
        );
    }

    #[test]
    fn parses_shifted_compact_gameplay_effect_record() {
        let mut record = vec![COMPACT_GAMEPLAY_EFFECT_MARKER_WITH_FIELD];
        record.extend_from_slice(&4579_u32.to_le_bytes());
        record.extend_from_slice(COMPACT_GAMEPLAY_EFFECT_FIELD);
        record.extend_from_slice(&5_u32.to_le_bytes());
        record.extend_from_slice(COMPACT_GAMEPLAY_EFFECT_TRAILER);
        let mut payload = vec![0; 56];
        write_shifted_bytes(&mut payload, 6, 20, &record);

        assert_eq!(
            parse_gameplay_effects(&payload),
            vec![ParsedGameplayEffect {
                unique_index: 4579,
                byte_offset: 21,
                bit_shift: 6,
            }]
        );
    }

    #[test]
    fn parses_bool_enum_sdk_compact_gameplay_effect_record() {
        let mut record = 4737_u32.to_le_bytes().to_vec();
        record.extend_from_slice(&[0, 124, 16, 0, 0, 0, 0, 0, 0, 0, 28, 32, 0, 0, 0]);
        let mut payload = vec![0; 40];
        write_shifted_bytes(&mut payload, 6, 12, &record);

        assert_eq!(
            parse_gameplay_effects(&payload),
            vec![ParsedGameplayEffect {
                unique_index: 4737,
                byte_offset: 12,
                bit_shift: 6,
            }]
        );
    }

    #[test]
    fn parses_compact_gameplay_effect_at_payload_boundary() {
        let mut payload = vec![COMPACT_GAMEPLAY_EFFECT_MARKER];
        payload.extend_from_slice(&3983_u32.to_le_bytes());
        payload.extend_from_slice(&9_u32.to_le_bytes());
        payload.extend_from_slice(COMPACT_GAMEPLAY_EFFECT_TRAILER);

        assert_eq!(
            parse_gameplay_effects(&payload),
            vec![ParsedGameplayEffect {
                unique_index: 3983,
                byte_offset: 1,
                bit_shift: 0,
            }]
        );
    }

    #[test]
    fn ignores_compact_gameplay_effect_without_structural_tail() {
        let mut payload = vec![COMPACT_GAMEPLAY_EFFECT_MARKER_WITH_FIELD];
        payload.extend_from_slice(&4579_u32.to_le_bytes());
        payload.extend_from_slice(COMPACT_GAMEPLAY_EFFECT_FIELD);
        payload.extend_from_slice(&5_u32.to_le_bytes());
        payload.extend_from_slice(&[58, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0]);

        assert!(parse_gameplay_effects(&payload).is_empty());
    }

    #[test]
    fn loads_gameplay_effect_names_from_assets() {
        let mapping =
            load_gameplay_effect_mapping(Path::new(GAMEPLAY_EFFECT_MAPPING_PATH)).unwrap();

        assert_eq!(
            mapping.get(&1012).map(String::as_str),
            Some("GE_Player_Sagiri_QTE1_Damage")
        );
    }

    #[test]
    fn loads_compact_gameplay_effect_names() {
        let path = write_temp_json(
            "compact_ge_mapping.json",
            r#"{"effects":{"1012":"GE_Player_Sagiri_QTE1_Damage"}}"#,
        );

        let mapping = load_gameplay_effect_mapping(&path).unwrap();

        assert_eq!(
            mapping.get(&1012).map(String::as_str),
            Some("GE_Player_Sagiri_QTE1_Damage")
        );
    }

    #[test]
    fn ability_tip_names_pick_field_by_active_language() {
        let path = write_temp_json(
            "compact_ability_tips.json",
            r#"{"abilities":{
                "GA_Mint019_Melee": {"name_zh": "满分收容术", "name_en": "Perfect Containment", "name_ja": "満点収容術"},
                "GA_NoGlobalName": {"name_zh": "仅中文技能"}
            }}"#,
        );

        let names = load_ability_tip_names(&path, Language::Japanese).unwrap();
        assert_eq!(
            names.get("GA_Mint019_Melee").map(String::as_str),
            Some("満点収容術")
        );
        // No Japanese (or English) translation for this ability — falls back
        // to whichever field is non-empty instead of dropping the name.
        assert_eq!(
            names.get("GA_NoGlobalName").map(String::as_str),
            Some("仅中文技能")
        );

        let names = load_ability_tip_names(&path, Language::English).unwrap();
        assert_eq!(
            names.get("GA_Mint019_Melee").map(String::as_str),
            Some("Perfect Containment")
        );

        let names = load_ability_tip_names(&path, Language::SimplifiedChinese).unwrap();
        assert_eq!(
            names.get("GA_Mint019_Melee").map(String::as_str),
            Some("满分收容术")
        );
    }

    #[test]
    #[cfg(not(feature = "external_resources"))]
    fn bundled_ability_tips_include_lingke_release_skills() {
        let names = load_ability_tip_names(
            Path::new("missing-root/res/data/skills/ability_tips.json"),
            Language::SimplifiedChinese,
        )
        .expect("bundled ability tips should load");

        assert_eq!(
            names.get("GA_Radio072_Skill").map(String::as_str),
            Some("变轨技能：瞬息全频振")
        );
        assert_eq!(
            names.get("GA_Radio072_UltraSkill").map(String::as_str),
            Some("超负荷共鸣")
        );
    }

    #[test]
    fn ability_tip_names_alias_jin_ultra_melee_to_its_named_parent() {
        let path = write_temp_json(
            "jin_ultra_melee_ability_tips.json",
            r#"{"abilities":{
                "GA_Jin_UltraSkill": {"name_zh": "浮世来潮", "name_en": "World's Tide", "name_ja": "浮世の波"},
                "GA_Jin_UltraSkill_Melee": {"name_zh": "", "name_en": "", "name_ja": ""}
            }}"#,
        );

        let names = load_ability_tip_names(&path, Language::SimplifiedChinese).unwrap();

        assert_eq!(
            names.get("GA_Jin_UltraSkill_Melee").map(String::as_str),
            Some("浮世来潮")
        );
    }

    #[test]
    fn loads_attack_types_from_skill_damage_assets() {
        let skills = load_gameplay_effect_skills(Path::new(SKILL_DAMAGE_DATA_PATH)).unwrap();

        for (effect, expected_type, expected_ability) in [
            (
                "GE_Player_Nanally_Melee1_Damage",
                "普攻",
                "GA_Nanally_Melee",
            ),
            (
                "GE_Player_Nanally_Skill1_Damage",
                "E技能",
                "GA_Nanally_Skill",
            ),
            (
                "GE_Player_Nanally_UltraSkill1_Damage",
                "Q技能",
                "GA_Nanally_UltraSkill",
            ),
            ("GE_Player_Sagiri_QTE1_Damage", "环合", "GA_Sagiri_QTE"),
            (
                "GE_Player_Nanally_PerfectEvadeAttack_Damage",
                "闪避反击",
                "GA_Nanally_ExtremEvadeAtk",
            ),
            (
                "GE_AbyssCard_T_004_Damage",
                "深渊场地Buff",
                "GA_CardTrigger_T_004",
            ),
            (
                "GE_AbyssCard_T_006_Damage",
                "深渊场地Buff",
                "GA_CardTrigger_T_006",
            ),
        ] {
            let skill = skills.get(effect).unwrap();
            assert_eq!(skill.attack_type, expected_type);
            assert_eq!(skill.ability_name.as_deref(), Some(expected_ability));
        }
    }

    #[test]
    fn loads_compact_skill_damage_index() {
        let path = write_temp_json(
            "compact_skill_damage.json",
            r#"{"skills":{"GE_Player_Nanally_Skill1_Damage":{"category":"E","ability":"GA_Nanally_Skill"}}}"#,
        );

        let skills = load_gameplay_effect_skills(&path).unwrap();
        let skill = skills.get("GE_Player_Nanally_Skill1_Damage").unwrap();

        assert_eq!(skill.damage_source_category.as_deref(), Some("E"));
        assert_eq!(skill.ability_name.as_deref(), Some("GA_Nanally_Skill"));
        assert_eq!(skill.attack_type, "E技能");
    }

    #[test]
    fn exact_gameplay_effect_semantics_override_parent_and_component() {
        let skills_path = write_temp_json(
            "semantic_skill_damage.json",
            r#"{"skills":{
                "GE_Player_Test_Special_Damage":{"category":"A","ability":"GA_Test_Melee"},
                "GE_Player_Test_Special_Damage_Adjacent":{"category":"A","ability":"GA_Test_Melee"}
            }}"#,
        );
        let semantics_path = write_temp_json(
            "semantic_effects.json",
            r#"{"format_version":1,"effects":{"GE_Player_Test_Special_Damage":{
                "owner_character_id":1055,
                "ability":"GA_Test_Passive_2",
                "attack_type":"Passive Damage",
                "damage_name_en":"Additional Settlement",
                "damage_name_zh":"追加清算",
                "damage_name_ja":"追加清算"
            }}}"#,
        );
        let mut catalog = AbilityCatalog::load(&skills_path).unwrap();

        catalog.apply_semantics(&semantics_path).unwrap();

        let skill = catalog.skill("GE_Player_Test_Special_Damage").unwrap();
        assert_eq!(skill.ability_name.as_deref(), Some("GA_Test_Passive_2"));
        assert_eq!(skill.attack_type, "Passive Damage");
        assert_eq!(
            skill.damage_component.as_deref(),
            Some("Additional Settlement")
        );
        assert_eq!(skill.owner_character_id, Some(1055));
        let adjacent = catalog
            .skill("GE_Player_Test_Special_Damage_Adjacent")
            .unwrap();
        assert_eq!(adjacent.ability_name.as_deref(), Some("GA_Test_Melee"));
        assert_eq!(adjacent.attack_type, "普攻");
        assert_eq!(adjacent.damage_component, None);
        assert_eq!(adjacent.owner_character_id, None);
        let names =
            load_gameplay_effect_semantic_names(&semantics_path, Language::SimplifiedChinese)
                .unwrap();
        assert_eq!(
            names.get("GE_Player_Test_Special_Damage"),
            Some(&("追加清算".to_owned(), true))
        );
    }

    #[test]
    fn gameplay_effect_semantics_reject_unknown_effect() {
        let skills_path = write_temp_json(
            "semantic_unknown_skill_damage.json",
            r#"{"skills":{"GE_Player_Test_Damage":{"category":"E","ability":"GA_Test_Skill"}}}"#,
        );
        let semantics_path = write_temp_json(
            "semantic_unknown_effect.json",
            r#"{"format_version":1,"effects":{"GE_Player_Missing_Damage":{
                "damage_name_en":"Missing",
                "damage_name_zh":"缺失",
                "damage_name_ja":"欠落"
            }}}"#,
        );
        let mut catalog = AbilityCatalog::load(&skills_path).unwrap();

        let error = catalog.apply_semantics(&semantics_path).unwrap_err();

        assert!(error.to_string().contains("GE_Player_Missing_Damage"));
    }

    #[test]
    fn loads_bundled_gameplay_effect_semantics() {
        let mut catalog = AbilityCatalog::load(Path::new(SKILL_DAMAGE_DATA_PATH)).unwrap();
        catalog
            .apply_semantics(Path::new(GAMEPLAY_EFFECT_SEMANTICS_PATH))
            .unwrap();

        let additional_settlement = catalog
            .skill("GE_Player_Kuhara_SeedReaction_Damage")
            .unwrap();
        assert_eq!(
            additional_settlement.ability_name.as_deref(),
            Some("GA_Kuhara_Passive_2")
        );
        assert_eq!(additional_settlement.attack_type, "Passive Damage");
        assert_eq!(
            additional_settlement.damage_component.as_deref(),
            Some("Additional Settlement")
        );
        assert_eq!(additional_settlement.owner_character_id, Some(1055));

        let blossom = catalog.skill("GE_ActorReaction_1_Damage").unwrap();
        assert_eq!(blossom.ability_name, None);
        assert_eq!(blossom.attack_type, "创生花");
        assert_eq!(blossom.damage_component.as_deref(), Some("Blossom Damage"));
        assert_eq!(blossom.owner_character_id, None);

        let replica_blossom = catalog.skill("GE_ActorReaction_1_1019_Damage").unwrap();
        assert_eq!(
            replica_blossom.ability_name.as_deref(),
            Some("GA_Oneiroi_Passive_1")
        );
        assert_eq!(replica_blossom.attack_type, "创生花");
        assert_eq!(
            replica_blossom.damage_component.as_deref(),
            Some("Replica Vita Pistil")
        );
        assert_eq!(replica_blossom.owner_character_id, None);

        for effect_name in [
            "GE_Player_Nanally_UltraSkill1_Damage",
            "GE_Player_Nanally_UltraSkill2_Damage",
            "GE_Player_Nanally_UltraSkill3_Damage",
        ] {
            let nanally_ultimate = catalog.skill(effect_name).unwrap();
            assert_eq!(nanally_ultimate.owner_character_id, Some(1010));
            assert_eq!(
                nanally_ultimate.ability_name.as_deref(),
                Some("GA_Nanally_UltraSkill")
            );
            assert_eq!(nanally_ultimate.damage_component, None);
        }

        let zankou_dot = catalog.skill("GE_Player_Zankou_DotDamage").unwrap();
        assert_eq!(zankou_dot.owner_character_id, Some(1036));
        assert_eq!(zankou_dot.ability_name.as_deref(), Some("GA_Zankou_Melee"));

        let names = load_gameplay_effect_semantic_names(
            Path::new(GAMEPLAY_EFFECT_SEMANTICS_PATH),
            Language::SimplifiedChinese,
        )
        .unwrap();
        assert_eq!(
            names.get("GE_ActorReaction_1_Damage"),
            Some(&("创生花".to_owned(), false))
        );
        assert_eq!(
            names.get("GE_ActorReaction_1_1019_Damage"),
            Some(&("复制创生花".to_owned(), true))
        );
        assert!(!names.contains_key("GE_Player_Nanally_UltraSkill3_Damage"));
    }

    #[test]
    fn classifies_qte_as_utf8_huanhe() {
        assert_eq!(
            classify_attack_type(None, "GE_Player_Test_QTE_Damage", Some("GA_Test_QTE")),
            "环合"
        );
    }

    #[test]
    fn normalizes_damage_names_for_grouping() {
        assert_eq!(normalize_damage_name("早雾大招2"), "早雾大招");
        assert_eq!(normalize_damage_name("早雾普攻2_1"), "早雾普攻");
        assert_eq!(normalize_damage_name("哈索尔QTE2"), "哈索尔环合");
        assert_eq!(normalize_damage_name("法帝娅大招追加3_2"), "法帝娅大招追加");
    }

    #[test]
    fn classifies_tenacity_and_parry_damage() {
        assert_eq!(
            classify_attack_type(Some("B"), "Buff_Tenacity_damage", None,),
            "倾陷伤害"
        );
        assert_eq!(
            classify_attack_type(Some("T"), "GE_Parry_Damage", None,),
            "格挡反击"
        );
        assert_eq!(
            classify_attack_type(None, "GE_Reaction_AnHunZhou", None,),
            "失谐"
        );
        assert_eq!(classify_attack_type(None, "GE_Reaction_7", None,), "盈蓄");
        assert_eq!(
            classify_attack_type(None, "GE_Reaction6_5Point_Damage", None,),
            "浸染"
        );
    }

    #[test]
    fn classifies_qte_reaction_from_participant_attributes() {
        assert_eq!(qte_reaction_type("暗", "咒"), Some("浊燃"));
        assert_eq!(qte_reaction_type("灵", "咒"), Some("覆纹"));

        assert_eq!(qte_reaction_type("光", "灵"), Some("创生"));
        assert_eq!(qte_reaction_type("光", "相"), Some("延滞"));

        assert_eq!(qte_reaction_type("暗", "咒"), Some("浊燃"));
        assert_eq!(qte_reaction_type("暗", "魂"), Some("黯星"));
        assert_eq!(qte_reaction_type("魂", "相"), Some("浸染"));
    }

    #[test]
    fn ignores_invalid_gameplay_effect_sentinel() {
        let mut payload = ACTIVE_GAMEPLAY_EFFECT_ANCHOR.to_vec();
        payload.extend_from_slice(&[0; ACTIVE_GAMEPLAY_EFFECT_VALUE_OFFSET]);
        payload.extend_from_slice(&ACTIVE_GAMEPLAY_EFFECT_MARKER.to_le_bytes());
        payload.extend_from_slice(&u32::MAX.to_le_bytes());

        assert!(parse_gameplay_effects(&payload).is_empty());
    }

    #[test]
    fn parses_target_handle_from_boss_hp_update() {
        let handle = client_fight_target_identity(0);
        let mut payload = vec![0_u8; 48];
        write_client_fight_array(&mut payload, 0, false, &[(handle, 1_927_891.0, 0)]);

        let updates = parse_boss_hp_updates(&payload);

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].target_handle, handle);
        assert_eq!(updates[0].current_hp, 1_927_891.0);
    }

    #[test]
    fn parses_client_fight_array_with_same_prefix_distinct_instances() {
        let first = client_fight_target_identity(0);
        let second = client_fight_target_identity(8);
        assert_eq!(first[..16], second[..16]);
        let mut payload = vec![0; 112];
        write_client_fight_array(
            &mut payload,
            3,
            false,
            &[(first, 200.0, 0), (second, 100.0, 0)],
        );

        let updates = parse_client_fight_target_updates(&payload);

        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].target_handle, first);
        assert_eq!(updates[0].current_hp, 200.0);
        assert_eq!(updates[1].target_handle, second);
        assert_eq!(updates[1].current_hp, 100.0);
        assert_ne!(updates[0].target_handle, updates[1].target_handle);
        assert_eq!(parse_boss_hp_updates(&payload), updates);
    }

    #[test]
    fn parses_server_damage_wrappers_for_each_client_fight_element() {
        let first = client_fight_target_identity(0);
        let second = client_fight_target_identity(8);
        let mut payload = vec![0_u8; 160];
        write_client_fight_damage_array(
            &mut payload,
            3,
            &[(first, 7_086.0, 1, 11_662, 0), (second, 1.0, 1, 6_934, 25)],
        );

        let settlements = parse_server_damage_settlements(&payload);

        assert_eq!(settlements.len(), 2);
        assert_eq!(settlements[0].target_handle, first);
        assert_eq!(settlements[0].current_hp, 7_086.0);
        assert_eq!(settlements[0].dead_state, 1);
        assert_eq!(settlements[0].raw_damage, 11_662);
        assert_eq!(settlements[0].secondary_value, 0);
        assert_eq!(settlements[1].target_handle, second);
        assert_eq!(settlements[1].raw_damage, 6_934);
        assert_eq!(settlements[1].secondary_value, 25);
    }

    #[test]
    fn rejects_one_point_or_wrongly_typed_server_damage_wrappers() {
        let target = client_fight_target_identity(0);
        let mut second = target;
        second[20] = 8;
        let mut one_point = vec![0_u8; 160];
        write_client_fight_damage_array(
            &mut one_point,
            5,
            &[(target, 100.0, 0, 1, 0), (second, 98.0, 0, 2, 0)],
        );
        let settlements = parse_server_damage_settlements(&one_point);
        assert_eq!(settlements.len(), 1);
        assert_eq!(settlements[0].target_handle, second);
        assert_eq!(settlements[0].raw_damage, 2);

        let mut wrong_type = vec![0_u8; 96];
        write_client_fight_damage_array(&mut wrong_type, 5, &[(target, 100.0, 0, 50, 0)]);
        write_bytes_at_bit(
            &mut wrong_type,
            5 + CLIENT_FIGHT_DATA_FIRST_WRAPPER_TYPE_BIT_OFFSET,
            &[0x05],
        );
        assert!(parse_server_damage_settlements(&wrong_type).is_empty());

        let mut positive_damage = vec![0_u8; 160];
        write_client_fight_damage_array(
            &mut positive_damage,
            5,
            &[(target, 100.0, 0, 2, 0), (second, 98.0, 0, 2, 0)],
        );
        write_bytes_at_bit(
            &mut positive_damage,
            5 + CLIENT_FIGHT_DATA_FIRST_WRAPPER_VALUE_BIT_OFFSET,
            &2_i32.to_le_bytes(),
        );
        assert!(parse_server_damage_settlements(&positive_damage).is_empty());
    }

    #[test]
    fn parses_current_sdk_compact_client_fight_instances() {
        let actor_value = [
            0x62, 0x01, 0x2d, 0x22, 0x05, 0xc8, 0x6d, 0x4f, 0x94, 0x61, 0x82, 0x17, 0xa4, 0x97,
            0x64, 0x72,
        ];
        let mut payload = vec![0_u8; 128];
        let mut first = [0_u8; COMPACT_CLIENT_FIGHT_RECORD_BYTES];
        first[..8].copy_from_slice(&BOSS_HP_PREFIX_HEAD);
        first[5..7].copy_from_slice(&0x40_u16.to_le_bytes());
        first[8..24].copy_from_slice(&actor_value);
        first[24..28].copy_from_slice(&2_u32.to_le_bytes());
        first[36..40].copy_from_slice(&69_613.0_f32.to_le_bytes());
        first[48..52].copy_from_slice(&1_u32.to_le_bytes());
        write_shifted_bytes(&mut payload, 3, 4, &first);

        let mut second = [0_u8; COMPACT_CLIENT_FIGHT_RECORD_BYTES];
        second[8..24].copy_from_slice(&actor_value);
        second[24..28].copy_from_slice(&5_u32.to_le_bytes());
        second[36..40].copy_from_slice(&327_350.0_f32.to_le_bytes());
        write_shifted_bytes(&mut payload, 3, 68, &second);

        let updates = parse_client_fight_target_updates(&payload);

        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].target_handle[..16], actor_value);
        assert_eq!(&updates[0].target_handle[16..20], &2_u32.to_le_bytes());
        assert_eq!(updates[0].current_hp, 69_613.0);
        assert_eq!(&updates[1].target_handle[16..20], &5_u32.to_le_bytes());
        assert_eq!(updates[1].current_hp, 327_350.0);
    }

    #[test]
    fn parses_only_sdk_confirmed_terminal_death_shape() {
        let target = client_fight_target_identity(8);
        let mut payload = vec![0; 48];
        write_client_fight_array(&mut payload, 5, true, &[(target, 0.0, 2)]);

        let updates = parse_client_fight_target_updates(&payload);

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].target_handle, target);
        assert_eq!(updates[0].current_hp, 0.0);

        write_bytes_at_bit(
            &mut payload,
            5 + CLIENT_FIGHT_DATA_DEAD_STATE_BIT_OFFSET,
            &1_u32.to_le_bytes(),
        );
        assert!(parse_client_fight_target_updates(&payload).is_empty());
    }

    #[test]
    fn rejects_truncated_or_oversized_client_fight_arrays() {
        let target = client_fight_target_identity(0);
        let mut truncated = vec![0; 48];
        write_client_fight_array(&mut truncated, 0, false, &[(target, 200.0, 0)]);
        truncated.truncate(20);
        assert!(parse_client_fight_target_updates(&truncated).is_empty());

        let mut oversized = vec![0; 48];
        let mut prefix = BOSS_HP_PREFIX_HEAD;
        prefix[5..7]
            .copy_from_slice(&(((MAX_CLIENT_FIGHT_DATA_ELEMENTS + 1) as u16) << 5).to_le_bytes());
        write_bytes_at_bit(&mut oversized, 0, &prefix);
        assert!(parse_client_fight_target_updates(&oversized).is_empty());
    }

    #[test]
    fn decodes_validated_client_damage_boss_wire_shape_at_fixed_offset() {
        let handle = client_fight_target_identity(0);
        let current_hp = 2_852_241.0_f32;
        let mut payload = vec![0; CLIENT_DAMAGE_BOSS_PAYLOAD_BITS.div_ceil(8)];
        write_client_fight_array(
            &mut payload,
            CLIENT_DAMAGE_BOSS_PREFIX_BYTE_OFFSET * 8 + usize::from(CLIENT_DAMAGE_BOSS_BIT_SHIFT),
            false,
            &[(handle, current_hp, 0)],
        );
        let packet = SequencedPacket {
            handler_prefix: 0,
            mode: 0,
            header_flags: 0,
            acknowledged_packet_id: 0,
            packet_id: 0,
            acknowledgment_history: 0,
            packet_flags: 0,
            payload_bit_len: CLIENT_DAMAGE_BOSS_PAYLOAD_BITS,
            payload,
        };
        let mut bunch_packet = BunchPacket {
            packet_info_bit_len: CLIENT_DAMAGE_BOSS_PACKET_INFO_BITS,
            bunches: vec![crate::engine::protocol::LocatedBunch {
                bit_offset: CLIENT_DAMAGE_BOSS_PACKET_INFO_BITS,
                bunch: crate::engine::protocol::SingleBunch {
                    prefix: 0,
                    sequence: 0,
                    descriptor: 0,
                    partial_flags: 0x0c,
                    data_bit_len: 91,
                    data: vec![0; 12],
                },
            }],
        };

        let update = parse_client_damage_boss_update(&packet, &bunch_packet)
            .expect("validated shape should decode");

        assert_eq!(update.target_handle, handle);
        assert_eq!(update.current_hp, current_hp);
        assert_eq!(update.byte_offset, 65);
        assert_eq!(update.bit_shift, 2);

        bunch_packet.bunches[0].bunch.descriptor = 1;
        assert!(parse_client_damage_boss_update(&packet, &bunch_packet).is_none());
    }

    #[test]
    fn parses_damage_in_tens_of_billions() {
        let damage = 50_000_000_000.0_f32;
        let payload = encoded_damage_record(damage, 90_000_000_000.0, 100_000_000_000.0);

        let records = parse_damage_records(&payload);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].damage, damage);
    }

    #[test]
    fn parses_bool_enum_damage_state_encoding() {
        let payload = encoded_bool_enum_damage_record_with_flags(
            26_043.0,
            1_224_273.0,
            1_224_273.0,
            [0, 1, 1],
        );

        let records = parse_damage_records(&payload);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].damage, 26_043.0);
        assert_eq!(records[0].state_flags, [0, 1, 1]);
        assert_eq!(records[0].encoding, DamageRecordEncoding::BoolAndEnums);
        assert_eq!(
            damage_record_encoding_at(&payload, records[0].byte_offset, records[0].bit_shift),
            Some(DamageRecordEncoding::BoolAndEnums)
        );
    }

    #[test]
    fn rejects_bool_enum_damage_record_with_invalid_bool_state() {
        let mut payload = encoded_bool_enum_damage_record_with_flags(
            26_043.0,
            1_224_273.0,
            1_224_273.0,
            [0, 1, 1],
        );
        let first_state_value_offset = (5 + 4) * 3 + (5 + 8) + (5 + 4) * 2 + 5;
        payload[first_state_value_offset] = 2;

        assert!(parse_damage_records(&payload).is_empty());
    }

    #[test]
    fn parses_bool_enum_damage_state_encoding_at_every_bit_shift() {
        let encoded = encoded_bool_enum_damage_record_with_flags(
            26_043.0,
            1_224_273.0,
            1_224_273.0,
            [0, 1, 1],
        );

        for bit_shift in 0..8 {
            let mut payload = vec![0; encoded.len() + 1];
            write_shifted_bytes(&mut payload, bit_shift, 0, &encoded);

            let records = parse_damage_records(&payload);

            assert_eq!(records.len(), 1, "bit shift {bit_shift}");
            assert_eq!(records[0].bit_shift, bit_shift);
            assert_eq!(records[0].state_flags, [0, 1, 1]);
            assert_eq!(records[0].encoding, DamageRecordEncoding::BoolAndEnums);
        }
    }

    #[test]
    fn keeps_large_damage_sanity_boundary() {
        let boundary = encoded_damage_record(MAX_COMBAT_VALUE, MAX_COMBAT_VALUE, MAX_COMBAT_VALUE);
        let above_boundary = encoded_damage_record(
            MAX_COMBAT_VALUE * 2.0,
            MAX_COMBAT_VALUE * 2.0,
            MAX_COMBAT_VALUE * 2.0,
        );

        assert!(parse_damage_record_at(&boundary, 0, 0).is_some());
        assert!(parse_damage_record_at(&above_boundary, 0, 0).is_none());
    }

    #[test]
    fn enforces_two_point_damage_floor() {
        let below_floor = encoded_damage_record(1.0, 45_126.0, 104_448.0);
        let at_floor = encoded_damage_record(2.0, 45_126.0, 104_448.0);

        assert!(parse_damage_records(&below_floor).is_empty());
        let records = parse_damage_records(&at_floor);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].damage, 2.0);
    }

    #[test]
    fn parses_boss_hp_in_tens_of_billions() {
        let current_hp = 80_000_000_000.0_f32;
        let mut payload = vec![0_u8; 48];
        write_client_fight_array(
            &mut payload,
            0,
            false,
            &[(client_fight_target_identity(0), current_hp, 0)],
        );

        let updates = parse_boss_hp_updates(&payload);

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].current_hp, current_hp);
    }

    #[test]
    fn incoming_damage_keeps_legacy_target_fields_empty() {
        let mut payload = b"/Game/Blueprints/Character/Monster/boss_07/".to_vec();
        payload.extend_from_slice(&encoded_damage_record(100.0, 1_500_000.0, 2_000_000.0));
        payload.extend_from_slice(&[5, 0, 0, 0, b'1', b'0', b'0', b'1', 0]);
        let evidence = find_declared_character_evidence(&payload);
        let characters = HashMap::from([(
            1001,
            CharacterInfo {
                name_zh: "Nanally".to_owned(),
                name_en: String::new(),
                color: None,
                avatar: None,
                attribute: None,
            },
        )]);

        let hits = parse_damage_payload(&payload, 1.0, Some(1001), None, &characters, &evidence);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].direction, HitDirection::Incoming);
        assert_eq!(hits[0].char_id, 1001);
        assert!(hits[0].target_id.is_none());
        assert!(hits[0].target_name.is_none());
        assert!(hits[0].target_context.is_empty());
        assert_eq!(
            hits[0].wire_event,
            Some(DamageWireEvent {
                damage: 100.0,
                target_hp_before: 1_500_000.0,
                target_max_hp: 2_000_000.0,
                damage_time: 1.0,
                world_time: 1.0,
                repeated_damage: 100.0,
                state_flags: [0, 0, 0],
                trailing_value: 0.0,
            })
        );
    }

    #[test]
    fn low_target_max_hp_without_matching_character_alignment_stays_outgoing() {
        let payload = encoded_damage_record(100.0, 10_000.0, 20_000.0);
        let evidence = [(1001, 1, 200)];
        let characters = HashMap::from([(
            1001,
            CharacterInfo {
                name_zh: "Nanally".to_owned(),
                name_en: String::new(),
                color: None,
                avatar: None,
                attribute: None,
            },
        )]);

        let hits = parse_damage_payload(&payload, 1.0, Some(1001), None, &characters, &evidence);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].direction, HitDirection::Outgoing);
        assert_eq!(hits[0].char_id, 1001);
        assert!(hits[0].target_id.is_none());
        assert!(hits[0].target_context.is_empty());
    }

    #[test]
    fn raw_damage_payload_does_not_fabricate_target_identity() {
        let payload = encoded_damage_record(1_341.0, 808_898.0, 808_898.0);
        let characters = HashMap::from([(
            1001,
            CharacterInfo {
                name_zh: "Nanally".to_owned(),
                name_en: String::new(),
                color: None,
                avatar: None,
                attribute: None,
            },
        )]);

        let hits = parse_damage_payload(&payload, 1.0, Some(1001), None, &characters, &[]);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].direction, HitDirection::Outgoing);
        assert!(hits[0].target_id.is_none());
        assert!(hits[0].target_context.is_empty());
    }

    #[test]
    fn sdk_adjacent_target_identity_reaches_the_damage_hit() {
        const TARGET_HANDLE_FROM_DAMAGE_BITS: usize = 1_277;

        let mut payload = encoded_damage_record(1_341.0, 808_898.0, 808_898.0);
        let target = client_fight_target_identity(8);
        let target_bit_offset = 5 * 8 + TARGET_HANDLE_FROM_DAMAGE_BITS;
        payload.resize(
            (target_bit_offset + CLIENT_FIGHT_TARGET_WIRE_BITS).div_ceil(8) + 1,
            0,
        );
        write_bytes_at_bit(&mut payload, target_bit_offset, &target);
        let characters = HashMap::from([(
            1001,
            CharacterInfo {
                name_zh: "Nanally".to_owned(),
                name_en: String::new(),
                color: None,
                avatar: None,
                attribute: None,
            },
        )]);

        let hits = parse_damage_payload(&payload, 1.0, Some(1001), None, &characters, &[]);

        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].target_id.as_deref(),
            Some(format!("enemy-wire:{}", hex::encode(target)).as_str())
        );
        assert_eq!(
            hits[0].target_context,
            [format!("enemy_target_wire={}", hex::encode(target))]
        );
    }

    #[test]
    fn current_sdk_compact_target_identity_reaches_the_damage_hit() {
        let mut payload = encoded_damage_record(1_341.0, 808_898.0, 808_898.0);
        let actor_value = [
            0x62, 0x01, 0x2d, 0x22, 0x05, 0xc8, 0x6d, 0x4f, 0x94, 0x61, 0x82, 0x17, 0xa4, 0x97,
            0x64, 0x72,
        ];
        let mut compact = [0_u8; COMPACT_CLIENT_FIGHT_IDENTITY_BYTES];
        compact[..16].copy_from_slice(&actor_value);
        compact[16..20].copy_from_slice(&5_u32.to_le_bytes());
        let target_bit_offset = 5 * 8 + COMPACT_DAMAGE_RECORD_TARGET_WIRE_BIT_DELTA;
        payload.resize(
            (target_bit_offset + COMPACT_CLIENT_FIGHT_IDENTITY_BYTES * 8).div_ceil(8) + 1,
            0,
        );
        write_bytes_at_bit(&mut payload, target_bit_offset, &compact);
        let characters = HashMap::from([(
            1001,
            CharacterInfo {
                name_zh: "Nanally".to_owned(),
                name_en: String::new(),
                color: None,
                avatar: None,
                attribute: None,
            },
        )]);

        let hits = parse_damage_payload(&payload, 1.0, Some(1001), None, &characters, &[]);

        let mut target = [0_u8; CLIENT_FIGHT_TARGET_WIRE_BYTES];
        target[..COMPACT_CLIENT_FIGHT_IDENTITY_BYTES].copy_from_slice(&compact);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].target_id.as_deref(),
            Some(format!("enemy-wire:{}", hex::encode(target)).as_str())
        );
    }

    #[test]
    fn sdk_adjacent_target_identity_accepts_confirmed_0x17_variant() {
        let mut payload = encoded_damage_record(1_341.0, 1_752_612.0, 1_752_612.0);
        let mut target = client_fight_target_identity(0);
        target[16] = 0x17;
        let target_bit_offset = 5 * 8 + BITPACKED_DAMAGE_RECORD_TARGET_WIRE_BIT_DELTA;
        payload.resize(
            (target_bit_offset + CLIENT_FIGHT_TARGET_WIRE_BITS).div_ceil(8) + 1,
            0,
        );
        write_bytes_at_bit(&mut payload, target_bit_offset, &target);
        let characters = HashMap::from([(
            1001,
            CharacterInfo {
                name_zh: "Nanally".to_owned(),
                name_en: String::new(),
                color: None,
                avatar: None,
                attribute: None,
            },
        )]);

        let hits = parse_damage_payload(&payload, 1.0, Some(1001), None, &characters, &[]);

        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].target_id.as_deref(),
            Some(format!("enemy-wire:{}", hex::encode(target)).as_str())
        );
    }

    #[test]
    fn incoming_state_flags_alone_do_not_mark_damage_as_incoming() {
        let payload = encoded_damage_record_with_flags(161.0, 5_388.0, 5_388.0, [0, 1, 0]);
        let evidence = [(1023, 6, 400)];
        let characters = HashMap::from([(
            1023,
            CharacterInfo {
                name_zh: "白藏".to_owned(),
                name_en: String::new(),
                color: None,
                avatar: None,
                attribute: None,
            },
        )]);

        let hits = parse_damage_payload(&payload, 1.0, Some(1023), None, &characters, &evidence);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].direction, HitDirection::Outgoing);
        assert_eq!(hits[0].char_id, 1023);
    }

    #[test]
    fn incoming_state_flags_do_not_mark_enemy_hp_as_incoming() {
        let payload =
            encoded_damage_record_with_flags(4_107.0, 1_053_085.5, 1_284_149.0, [0, 1, 0]);
        let evidence = [(1004, 1, 200)];
        let characters = HashMap::from([(
            1004,
            CharacterInfo {
                name_zh: "安魂曲".to_owned(),
                name_en: String::new(),
                color: None,
                avatar: None,
                attribute: None,
            },
        )]);

        let hits = parse_damage_payload(&payload, 1.0, Some(1004), None, &characters, &evidence);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].direction, HitDirection::Outgoing);
        assert_eq!(hits[0].char_id, 1004);
    }

    #[test]
    fn outgoing_state_flags_do_not_mark_damage_as_incoming_without_target_anchor() {
        let payload =
            encoded_damage_record_with_flags(1_604.0, 1_340_986.0, 1_930_389.0, [0, 1, 1]);
        let evidence = [(1001, 1, 200)];
        let characters = HashMap::from([(
            1001,
            CharacterInfo {
                name_zh: "Nanally".to_owned(),
                name_en: String::new(),
                color: None,
                avatar: None,
                attribute: None,
            },
        )]);

        let hits = parse_damage_payload(&payload, 1.0, Some(1001), None, &characters, &evidence);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].direction, HitDirection::Outgoing);
        assert_eq!(hits[0].char_id, 1001);
    }

    #[test]
    fn same_shift_character_anchor_before_damage_record_stays_outgoing() {
        let payload = encoded_damage_record(100.0, 10_000.0, 20_000.0);
        let evidence = [(1001, 0, 0)];
        let characters = HashMap::from([(
            1001,
            CharacterInfo {
                name_zh: "Nanally".to_owned(),
                name_en: String::new(),
                color: None,
                avatar: None,
                attribute: None,
            },
        )]);

        let hits = parse_damage_payload(&payload, 1.0, Some(1001), None, &characters, &evidence);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].direction, HitDirection::Outgoing);
        assert_eq!(hits[0].char_id, 1001);
    }

    #[test]
    fn resolves_character_from_matching_bit_alignment() {
        let evidence = [(1003, 2, 241), (1004, 3, 50), (1003, 0, 606)];

        assert_eq!(declared_character_for_shift(&evidence, 3), Some(1004));
        assert_eq!(declared_character_for_shift(&evidence, 2), Some(1003));
        assert_eq!(declared_character_for_shift(&evidence, 5), None);
    }

    #[test]
    fn final_tower_character_evidence_requires_character_for_net_anchor() {
        let payload = b"ft_character_1076".to_vec();

        assert!(find_final_tower_character_evidence(&payload).is_empty());
    }

    #[test]
    fn finds_final_tower_character_evidence() {
        let payload = b"FCharacterForNet....ft_character_1076".to_vec();

        assert_eq!(
            find_final_tower_character_evidence(&payload),
            vec![(1076, 0, 20)]
        );
    }

    #[test]
    fn finds_shifted_final_tower_character_evidence() {
        let mut payload = vec![0_u8; 64];
        write_shifted_bytes(&mut payload, 5, 7, b"FCharacterForNet");
        write_shifted_bytes(&mut payload, 5, 30, b"ft_character_1076");

        assert_eq!(
            find_final_tower_character_evidence(&payload),
            vec![(1076, 5, 30)]
        );
    }

    #[test]
    fn final_tower_character_evidence_ignores_longer_numeric_suffix() {
        let payload = b"FCharacterForNet....ft_character_10760".to_vec();

        assert!(find_final_tower_character_evidence(&payload).is_empty());
    }
}

#[cfg(test)]
mod empty_curtain_tests {
    use super::*;

    #[derive(Default)]
    struct BitWriter {
        data: Vec<u8>,
        bit_len: usize,
    }

    impl BitWriter {
        fn push_bits(&mut self, value: u64, count: usize) {
            let new_bit_len = self.bit_len + count;
            self.data.resize(new_bit_len.div_ceil(8), 0);
            for index in 0..count {
                let target = self.bit_len + index;
                self.data[target / 8] |= (((value >> index) & 1) as u8) << (target % 8);
            }
            self.bit_len = new_bit_len;
        }

        fn push_bool(&mut self, value: bool) {
            self.push_bits(u64::from(value), 1);
        }

        fn push_u16(&mut self, value: u16) {
            self.push_bits(u64::from(value), 16);
        }

        fn push_u32(&mut self, value: u32) {
            self.push_bits(u64::from(value), 32);
        }

        fn push_i32(&mut self, value: i32) {
            self.push_u32(value as u32);
        }

        fn push_i64(&mut self, value: i64) {
            self.push_bits(value as u64, 64);
        }

        fn push_f32(&mut self, value: f32) {
            self.push_u32(value.to_bits());
        }

        fn push_fstring(&mut self, value: &str) {
            self.push_i32((value.len() + 1) as i32);
            for byte in value.bytes() {
                self.push_bits(u64::from(byte), 8);
            }
            self.push_bits(0, 8);
        }

        fn push_dynamic_name(&mut self, value: &str) {
            self.push_bool(false);
            self.push_fstring(value);
            self.push_u32(0);
        }
    }

    fn push_compact_module_cell(
        writer: &mut BitWriter,
        item_id: &str,
        id: HtItemNetId,
        first_step: bool,
        row: i32,
        column: i32,
    ) {
        writer.push_i32(1);
        writer.push_dynamic_name(item_id);
        writer.push_u32(id.solt);
        writer.push_u32(id.serial);
        writer.push_bool(first_step);
        writer.push_i32(row);
        writer.push_i32(column);
        writer.push_i32(0);
    }

    fn push_module_record(
        writer: &mut BitWriter,
        id: HtItemNetId,
        character_id: HtItemNetId,
        locked: bool,
        discarded: bool,
        equipped_tail: i32,
        equipment_count: u16,
    ) {
        writer.push_dynamic_name("cell3_style1_1_Orange");
        writer.push_u32(id.solt);
        writer.push_u32(id.serial);
        writer.push_i64(1);
        writer.push_i32(0);
        writer.push_i64(1);
        writer.push_u16(0);
        writer.push_u16(0);
        writer.push_u16(equipment_count);
        writer.push_i32(20);
        writer.push_u32(character_id.solt);
        writer.push_u32(character_id.serial);
        writer.push_i32(0);
        writer.push_bool(locked);
        writer.push_bool(discarded);
        writer.push_u16(0);
        writer.push_u16(2);
        writer.push_dynamic_name("AtkAdd");
        writer.push_dynamic_name("HPMaxAdd");
        writer.push_u16(4);
        for (property, value) in [
            ("DefUp", 0.0525),
            ("AtkAdd", 24.0),
            ("CritDamageBase", 0.06),
            ("CritBase", 0.03),
        ] {
            writer.push_dynamic_name(property);
            writer.push_f32(value);
        }
        writer.push_f32(0.0);
        writer.push_bool(false);
        writer.push_i32(equipped_tail);
        writer.push_i32(0);
    }

    fn push_character_record_prefix(
        writer: &mut BitWriter,
        item_id: &str,
        net_id: HtItemNetId,
        character_count: u16,
    ) {
        writer.push_dynamic_name(item_id);
        writer.push_u32(net_id.solt);
        writer.push_u32(net_id.serial);
        writer.push_i64(1);
        writer.push_i32(0);
        writer.push_i64(1);
        writer.push_u16(character_count);
        if character_count == 0 {
            return;
        }
        writer.push_i32(80);
        writer.push_i32(6);
        writer.push_u32(100);
        writer.push_u32(200);
        writer.push_i32(6);
        for value in [1.0, 20_000.0, 3.0, 120.0, 80.0, 1_000.0, 100.0] {
            writer.push_f32(value);
        }
        writer.push_bool(false);
        writer.push_i32(0);
        writer.push_u16(5);
    }

    fn catalog() -> EquipmentCatalog {
        load_equipment_catalog(Path::new(EQUIPMENT_CATALOG_PATH))
            .expect("bundled equipment catalog should load")
    }

    #[test]
    fn loads_equipment_catalog_and_interpolates_main_stats() {
        let catalog = catalog();
        let item = &catalog.items["cell3_style1_1_Orange"];

        assert_eq!(catalog.items.len(), 74);
        assert_eq!(catalog.attributes.len(), 53);
        assert_eq!(catalog.curves.len(), 74);
        assert_eq!(catalog.suits.len(), 12);
        assert_eq!(catalog.shapes.len(), 12);
        assert_eq!(catalog.plans.len(), 21);
        assert_eq!(catalog.main_stat_value(item, "AtkAdd", 20), Some(63.0));
        assert_eq!(catalog.main_stat_value(item, "HPMaxAdd", 20), Some(840.0));
    }

    #[test]
    fn module_positions_apply_character_template_and_shape_in_business_layer() {
        let catalog = catalog();
        let positions = catalog
            .valid_module_positions(1076, "cell4_style1_1_Orange")
            .expect("1076 and Hen4 must have equipment metadata");

        assert!(positions.contains(&EquipmentGridCell { row: 2, column: 3 }));
        assert!(!positions.contains(&EquipmentGridCell { row: 1, column: 1 }));
        assert!(!positions.contains(&EquipmentGridCell { row: 2, column: 4 }));
    }

    #[test]
    fn equipment_catalog_rejects_card_overflow_and_non_numeric_suit_ids() {
        let mut too_many_rows = catalog();
        let item = too_many_rows
            .items
            .values_mut()
            .next()
            .expect("bundled equipment catalog must contain items");
        item.main_count = EMPTY_CURTAIN_MAX_STAT_ROWS;
        item.sub_count = 1;
        assert!(validate_equipment_catalog(&too_many_rows).is_err());

        let mut non_numeric_suit = catalog();
        let suit = non_numeric_suit
            .suits
            .remove("Suit1")
            .expect("bundled equipment catalog must contain Suit1");
        non_numeric_suit.suits.insert("DamageSet".to_owned(), suit);
        for item in non_numeric_suit.items.values_mut() {
            if item.suit.as_deref() == Some("Suit1") {
                item.suit = Some("DamageSet".to_owned());
            }
        }
        assert!(validate_equipment_catalog(&non_numeric_suit).is_err());
    }

    #[test]
    fn parses_locked_discarded_and_equipped_item_from_character_net_id() {
        let mut writer = BitWriter::default();
        push_module_record(
            &mut writer,
            HtItemNetId {
                solt: 1_117_099_843,
                serial: 211_541_362,
            },
            HtItemNetId {
                solt: 1_135_129_303,
                serial: 1_041_749_587,
            },
            true,
            true,
            0,
            1,
        );

        let catalog = catalog();
        let mut items = parse_empty_curtain_items(&writer.data, writer.bit_len, &catalog);

        assert_eq!(items.len(), 1);
        assert!(items[0].locked);
        assert!(items[0].discarded);
        assert!(items[0].is_equipped());
        assert_eq!(items[0].main_stats[0].value, 63.0);
        assert_eq!(items[0].main_stats[1].value, 840.0);
        assert_eq!(items[0].sub_stats.len(), 4);
        assert!(validate_empty_curtain_snapshot(&items, &catalog));

        items[0].equipped_placement =
            Some(crate::engine::model::EmptyCurtainPlacement { row: 1, column: 2 });
        assert!(validate_empty_curtain_snapshot(&items, &catalog));
        items[0].equipped_placement =
            Some(crate::engine::model::EmptyCurtainPlacement { row: 7, column: 2 });
        assert!(!validate_empty_curtain_snapshot(&items, &catalog));
    }

    #[test]
    fn parses_remove_notification_items_without_treating_other_notifications_as_removals() {
        let first_id = HtItemNetId {
            solt: 1_117_099_843,
            serial: 211_541_362,
        };
        let second_id = HtItemNetId {
            solt: 1_117_099_844,
            serial: 211_541_363,
        };
        let mut writer = BitWriter::default();
        writer.push_bits(0x12ab, 13);
        writer.push_bits(2, 7);
        writer.push_bits(2, 17);
        push_module_record(&mut writer, first_id, HtItemNetId::ZERO, false, false, 0, 1);
        push_module_record(
            &mut writer,
            second_id,
            HtItemNetId::ZERO,
            false,
            false,
            0,
            1,
        );

        assert_eq!(
            parse_empty_curtain_item_removals(&writer.data, writer.bit_len, &catalog()),
            vec![
                (first_id, "cell3_style1_1_Orange".to_owned()),
                (second_id, "cell3_style1_1_Orange".to_owned()),
            ]
        );

        let mut writer = BitWriter::default();
        writer.push_bits(1, 7);
        writer.push_bits(1, 17);
        push_module_record(&mut writer, first_id, HtItemNetId::ZERO, false, false, 0, 1);
        assert!(
            parse_empty_curtain_item_removals(&writer.data, writer.bit_len, &catalog()).is_empty()
        );

        let mut writer = BitWriter::default();
        writer.push_bits(2, 7);
        writer.push_bits(3, 17);
        push_module_record(&mut writer, first_id, HtItemNetId::ZERO, false, false, 0, 1);
        assert!(
            parse_empty_curtain_item_removals(&writer.data, writer.bit_len, &catalog()).is_empty(),
            "a truncated removal batch must not delete the records parsed before its missing tail"
        );
    }

    #[test]
    fn parses_complete_prefix_of_a_truncated_add_notification() {
        let first_id = HtItemNetId {
            solt: 1_117_099_843,
            serial: 211_541_362,
        };
        let second_id = HtItemNetId {
            solt: 1_117_099_844,
            serial: 211_541_363,
        };
        let mut writer = BitWriter::default();
        writer.push_bits(3, 7);
        writer.push_bits(3, 17);
        push_module_record(&mut writer, first_id, HtItemNetId::ZERO, false, false, 0, 1);
        push_module_record(
            &mut writer,
            second_id,
            HtItemNetId::ZERO,
            false,
            false,
            0,
            1,
        );

        let additions =
            parse_empty_curtain_item_additions(&writer.data, writer.bit_len, &catalog());

        assert_eq!(
            additions
                .into_iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            [first_id, second_id]
        );
    }

    #[test]
    fn parses_character_instance_to_template_mapping() {
        let mut writer = BitWriter::default();
        let net_id = HtItemNetId {
            solt: 1_135_129_303,
            serial: 1_041_749_587,
        };
        push_character_record_prefix(&mut writer, "1020", net_id, 1);

        let owners = parse_empty_curtain_character_owners(&writer.data, writer.bit_len);

        assert_eq!(owners, HashMap::from([(net_id, 1020)]));
    }

    #[test]
    fn parses_compact_login_module_placement() {
        let catalog = catalog();
        let id = HtItemNetId {
            solt: 1_296_894_783,
            serial: 538_340_435,
        };
        let mut writer = BitWriter::default();
        push_compact_module_cell(&mut writer, "cell2_style2_1_Orange", id, true, 1, 5);
        push_compact_module_cell(&mut writer, "cell2_style2_1_Orange", id, false, 2, 5);

        assert_eq!(
            parse_empty_curtain_compact_module_placements(
                &writer.data,
                writer.bit_len,
                &HashMap::new(),
                &catalog,
            ),
            vec![ParsedEmptyCurtainCompactModulePlacement {
                equipment: id,
                item_id: "cell2_style2_1_Orange".to_owned(),
                row: 1,
                column: 5,
            }]
        );
    }

    #[test]
    fn compact_login_module_parser_rejects_incomplete_geometry() {
        let catalog = catalog();
        let id = HtItemNetId {
            solt: 1_296_894_783,
            serial: 538_340_435,
        };
        let mut writer = BitWriter::default();
        push_compact_module_cell(&mut writer, "cell2_style2_1_Orange", id, true, 1, 5);

        assert!(
            parse_empty_curtain_compact_module_placements(
                &writer.data,
                writer.bit_len,
                &HashMap::new(),
                &catalog,
            )
            .is_empty()
        );
    }

    #[test]
    fn compact_login_module_parser_rejects_uid_item_mismatch() {
        let catalog = catalog();
        let id = HtItemNetId {
            solt: 1_296_894_783,
            serial: 538_340_435,
        };
        let mut writer = BitWriter::default();
        push_compact_module_cell(&mut writer, "cell2_style2_1_Orange", id, true, 1, 5);
        push_compact_module_cell(&mut writer, "cell2_style2_1_Orange", id, false, 2, 5);
        let known_items = HashMap::from([(
            id,
            EmptyCurtainItem {
                id,
                item_id: "cell2_style1_1_Orange".to_owned(),
                level: 0,
                main_stats: Vec::new(),
                sub_stats: Vec::new(),
                locked: false,
                discarded: false,
                character_net_id: None,
                equipped_character_id: None,
                equipped_placement: None,
            },
        )]);

        assert!(
            parse_empty_curtain_compact_module_placements(
                &writer.data,
                writer.bit_len,
                &known_items,
                &catalog,
            )
            .is_empty()
        );
    }

    #[test]
    fn character_owner_parser_rejects_truncated_prefix() {
        let mut writer = BitWriter::default();
        push_character_record_prefix(
            &mut writer,
            "1020",
            HtItemNetId {
                solt: 10,
                serial: 20,
            },
            1,
        );

        assert!(parse_empty_curtain_character_owners(&writer.data, writer.bit_len - 1).is_empty());
    }

    #[test]
    fn character_owner_parser_ignores_non_character_item_shapes() {
        let mut non_numeric = BitWriter::default();
        push_character_record_prefix(
            &mut non_numeric,
            "cell3_style1_1_Orange",
            HtItemNetId {
                solt: 10,
                serial: 20,
            },
            1,
        );
        let mut no_character_payload = BitWriter::default();
        push_character_record_prefix(
            &mut no_character_payload,
            "1020",
            HtItemNetId {
                solt: 10,
                serial: 20,
            },
            0,
        );

        assert!(
            parse_empty_curtain_character_owners(&non_numeric.data, non_numeric.bit_len).is_empty()
        );
        assert!(
            parse_empty_curtain_character_owners(
                &no_character_payload.data,
                no_character_payload.bit_len
            )
            .is_empty()
        );
    }

    #[test]
    fn rejects_truncated_or_wrong_array_shape() {
        let catalog = catalog();
        let mut valid = BitWriter::default();
        push_module_record(
            &mut valid,
            HtItemNetId {
                solt: 10,
                serial: 20,
            },
            HtItemNetId { solt: 0, serial: 0 },
            false,
            false,
            0,
            1,
        );
        assert!(parse_empty_curtain_items(&valid.data, valid.bit_len - 1, &catalog).is_empty());

        let mut wrong_shape = BitWriter::default();
        push_module_record(
            &mut wrong_shape,
            HtItemNetId {
                solt: 10,
                serial: 20,
            },
            HtItemNetId { solt: 0, serial: 0 },
            false,
            false,
            0,
            0,
        );
        assert!(
            parse_empty_curtain_items(&wrong_shape.data, wrong_shape.bit_len, &catalog).is_empty()
        );
    }

    #[test]
    fn ignores_equipped_tail_and_keeps_distinct_instance_ids() {
        let mut writer = BitWriter::default();
        for id in [
            HtItemNetId {
                solt: 10,
                serial: 20,
            },
            HtItemNetId {
                solt: 11,
                serial: 21,
            },
        ] {
            push_module_record(
                &mut writer,
                id,
                HtItemNetId { solt: 0, serial: 0 },
                false,
                false,
                1,
                1,
            );
        }

        let items = parse_empty_curtain_items(&writer.data, writer.bit_len, &catalog());

        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| !item.is_equipped()));
    }
}
