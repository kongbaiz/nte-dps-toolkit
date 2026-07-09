use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::engine::model::{CharacterInfo, Hit};
use crate::storage::i18n::Language;
use crate::storage::resource::{read_resource_text, resource_exists, resource_file_path};

const RECORD_FIELD_TYPES: [u8; 10] = [12, 12, 12, 13, 12, 12, 6, 6, 6, 12];
const RECORD_FIELD_LENGTHS: [usize; 10] = [4, 4, 4, 8, 4, 4, 4, 4, 4, 4];
const MAX_RECORD_FIELD_LENGTH: usize = 8;
const MIN_DAMAGE: f32 = 2.0;
const MAX_DAMAGE: f32 = 1_000_000_000.0;
const MAX_PLAUSIBLE_CURRENT_HP_UPDATE: f32 = 500_000.0;
const CURRENT_HP_PREFIX_LENGTH: usize = 16;
const BOSS_HP_PREFIX_LENGTH: usize = 36;
const BOSS_HP_PREFIX_HEAD: [u8; 8] = [0x06, 0x00, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00];
const ACTIVE_GAMEPLAY_EFFECT_ANCHOR: &[u8] = b"FHTClientActiveGE";
const ACTIVE_GAMEPLAY_EFFECT_VALUE_OFFSET: usize = 5;
const ACTIVE_GAMEPLAY_EFFECT_MARKER: u32 = 12;
const EQUIPMENT_SLOT_STATE_ANCHOR: &[u8] = b"\x06\0\0\0State\0";
const MAX_TAGGED_PROPERTY_STRING_LENGTH: i32 = 256;

pub const CHARACTER_DATA_PATH: &str = "res/data/characters/characters.json";
pub const GAMEPLAY_EFFECT_MAPPING_PATH: &str = "res/data/skills/gameplay_effect_mapping.json";
pub const SKILL_DAMAGE_DATA_PATH: &str = "res/data/skills/skill_damage.json";
pub const ULTRA_TIME_STOP_DATA_PATH: &str = "res/data/skills/ultra_time_stop.json";
pub const WOODEN_DAMAGE_DESCRIPTIONS_PATH: &str = "res/data/skills/wooden_damage_descriptions.json";
pub const ABILITY_TIPS_PATH: &str = "res/data/skills/ability_tips.json";

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
    pub byte_offset: usize,
    pub bit_shift: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedCurrentHpUpdate {
    pub current_hp: f32,
    pub byte_offset: usize,
    pub bit_shift: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedBossHpUpdate {
    pub target_handle: [u8; 16],
    pub current_hp: f32,
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
pub struct GameplayEffectSkill {
    pub damage_source_category: Option<String>,
    pub ability_name: Option<String>,
    pub attack_type: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct UltraTimeStopEntry {
    #[serde(default)]
    pub ability_id: String,
    #[serde(default)]
    pub end_ability_event_seconds: f64,
    #[serde(default)]
    pub extra_cooldowns: Vec<UltraTimeStopCooldown>,
    #[serde(default)]
    pub ignored_cooldown_tags: Vec<String>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub confidence: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct UltraTimeStopCooldown {
    #[serde(default)]
    pub cooldown_tag: String,
    #[serde(default)]
    pub ability_id: String,
    #[serde(default)]
    pub duration_seconds: f64,
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
                },
            )
        })
        .collect())
}

#[derive(Deserialize)]
struct UltraTimeStopDocument {
    characters: HashMap<String, UltraTimeStopEntry>,
}

pub fn load_ultra_time_stops(path: &Path) -> Result<HashMap<u32, UltraTimeStopEntry>> {
    let text = read_resource_text(path)
        .with_context(|| format!("无法读取大招时停表 {}", path.display()))?;
    let document: UltraTimeStopDocument =
        serde_json::from_str(&text).context("大招时停表 JSON 无效")?;
    Ok(document
        .characters
        .into_iter()
        .filter_map(|(key, value)| {
            key.parse::<u32>()
                .ok()
                .filter(|_| {
                    value.end_ability_event_seconds.is_finite()
                        && value.end_ability_event_seconds > 0.0
                })
                .map(|id| (id, value))
        })
        .collect())
}

// Superseded by ability_tips-based name resolution (load_ability_tip_names);
// kept only as a regression test for the legacy wooden-dummy description
// parsing below via loads_chinese_damage_names_from_wooden_assets /
// loads_compact_wooden_damage_names.
#[cfg(test)]
fn load_wooden_damage_names(path: &Path) -> Result<HashMap<String, String>> {
    let text = read_resource_text(path)
        .with_context(|| format!("无法读取木桩伤害描述表 {}", path.display()))?;
    let document: serde_json::Value =
        serde_json::from_str(&text).context("木桩伤害描述表 JSON 无效")?;
    if let Some(names) = document.get("names").and_then(serde_json::Value::as_object) {
        return Ok(names
            .iter()
            .filter_map(|(effect_name, name)| {
                name.as_str()
                    .filter(|name| !name.trim().is_empty())
                    .map(|name| (effect_name.clone(), name.to_owned()))
            })
            .collect());
    }
    let rows = document
        .as_array()
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.get("Rows"))
        .and_then(serde_json::Value::as_object)
        .context("木桩伤害描述表缺少 Rows")?;

    Ok(rows
        .iter()
        .filter_map(|(effect_name, row)| {
            row.get("Desc")
                .and_then(|desc| desc.get("CultureInvariantString"))
                .and_then(serde_json::Value::as_str)
                .filter(|description| !description.trim().is_empty())
                .map(|description| (effect_name.clone(), normalize_damage_name(description)))
        })
        .collect())
}

/// Maps GA_ ability names to their official in-game skill name, sourced from
/// `DT_GameplayAbilityTipsData`. Unlike the wooden dummy descriptions this table is
/// kept current with new characters, but it is keyed by ability rather than by
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
    Ok(abilities
        .iter()
        .filter_map(|(ability_name, row)| {
            let name = field_priority.iter().find_map(|field| {
                row.get(*field)
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.trim().is_empty())
            })?;
            Some((ability_name.clone(), name.trim().to_owned()))
        })
        .collect())
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

pub fn classify_attack_type_from_description(description: &str) -> Option<String> {
    if description.contains("QTE") || description.contains("环合") {
        Some("环合".to_owned())
    } else if description.contains("大招") {
        Some("Q技能".to_owned())
    } else if description.contains("普攻") {
        Some("普攻".to_owned())
    } else if description.contains("技能") {
        Some("E技能".to_owned())
    } else {
        None
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

fn read_property_header<'a>(
    data: &'a [u8],
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

fn parse_damage_record_at(
    data: &[u8],
    byte_offset: usize,
    bit_shift: u8,
) -> Option<ParsedDamageRecord> {
    let mut fields = [Field::default(); RECORD_FIELD_TYPES.len()];
    let mut bit_cursor = 0;
    for (index, (expected_type, expected_length)) in RECORD_FIELD_TYPES
        .into_iter()
        .zip(RECORD_FIELD_LENGTHS)
        .enumerate()
    {
        let (field_type, field, consumed) = read_field(data, byte_offset, bit_shift, bit_cursor)?;
        if field_type != expected_type || field.len != expected_length {
            return None;
        }
        bit_cursor += consumed;
        fields[index] = field;
    }

    let damage = f32_field(&fields[0])?;
    let target_hp_before = f32_field(&fields[1])?;
    let target_max_hp = f32_field(&fields[2])?;
    let damage_time = f64_field(&fields[3])?;
    let world_time = f32_field(&fields[4])?;
    let repeated_damage = f32_field(&fields[5])?;
    let state_flags = [
        i32_field(&fields[6])?,
        i32_field(&fields[7])?,
        i32_field(&fields[8])?,
    ];
    let trailing_value = f32_field(&fields[9])?;

    if !damage.is_finite()
        || !(MIN_DAMAGE..=MAX_DAMAGE).contains(&damage)
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
        byte_offset: byte_offset + 5,
        bit_shift,
    })
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

pub fn parse_boss_hp_updates(data: &[u8]) -> Vec<ParsedBossHpUpdate> {
    let mut updates = Vec::new();
    for byte_offset in 0..data.len() {
        for bit_shift in 0..8_u8 {
            let mut decoded = [0; BOSS_HP_PREFIX_LENGTH + 4];
            if decode_shifted_into(data, byte_offset, bit_shift, 0, &mut decoded).is_none()
                || decoded[..BOSS_HP_PREFIX_HEAD.len()] != BOSS_HP_PREFIX_HEAD
                || decoded[8..24].iter().all(|byte| *byte == 0)
                || decoded[24..BOSS_HP_PREFIX_LENGTH]
                    .iter()
                    .any(|byte| *byte != 0)
            {
                continue;
            }
            let current_hp = f32::from_le_bytes(
                decoded[BOSS_HP_PREFIX_LENGTH..]
                    .try_into()
                    .expect("Boss HP field has a fixed four-byte length"),
            );
            if !current_hp.is_finite() || !(0.0..=MAX_DAMAGE).contains(&current_hp) {
                continue;
            }
            updates.push(ParsedBossHpUpdate {
                target_handle: decoded[8..24]
                    .try_into()
                    .expect("Boss target handle has a fixed 16-byte length"),
                current_hp,
                byte_offset: byte_offset + BOSS_HP_PREFIX_LENGTH,
                bit_shift,
            });
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
            "incoming"
        } else if resolved_packet_char_id.is_some() {
            "outgoing"
        } else {
            "unknown"
        };
        hits.push(Hit {
            timestamp,
            char_id,
            char_name: name,
            char_known: character.is_some(),
            damage: damage as f64,
            byte_offset,
            bit_shift,
            char_source: if resolved_packet_char_id.is_some() {
                "packet"
            } else if fallback_char_id.is_some() {
                "session"
            } else {
                "unknown"
            }
            .to_owned(),
            direction: direction.to_owned(),
            target_hp_before: target_hp_before as f64,
            target_hp_after: target_hp_after as f64,
            target_max_hp: target_max_hp as f64,
            target_hp_percent: if target_max_hp > 0.0 {
                target_hp_after as f64 / target_max_hp as f64 * 100.0
            } else {
                0.0
            },
            target_id: None,
            target_name: None,
            target_context: Vec::new(),
            gameplay_effect_index: None,
            gameplay_effect_name: None,
            ability_name: None,
            damage_name: None,
            attack_type: None,
            damage_attribute: None,
            follow_up_damage: 0.0,
            follow_up_timestamp: None,
            follow_up_damage_name: None,
            follow_up_attack_type: None,
            follow_up_damage_attribute: None,
        });
    }
    hits
}

#[cfg(test)]
mod character_tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEMP_JSON_COUNTER: AtomicUsize = AtomicUsize::new(0);

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

    fn encoded_equipment_slot(
        equipment_id: &str,
        solt: u32,
        serial: u32,
        row: i32,
        column: i32,
    ) -> Vec<u8> {
        let mut payload = Vec::new();
        push_i32_property(&mut payload, "State", 1);
        push_name_property(&mut payload, "EquipmentID", equipment_id);
        push_ht_item_net_id_property(&mut payload, solt, serial);
        push_bool_property(&mut payload, "bFirstStep", 0x10);
        push_i32_property(&mut payload, "Row", row);
        push_i32_property(&mut payload, "Column", column);
        push_i32_property(&mut payload, "New", 0);
        push_none_property(&mut payload);
        payload
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
    fn load_characters_falls_back_to_bundled_resource() {
        let characters = load_characters(Path::new(
            "missing-root/res/data/characters/characters.json",
        ))
        .expect("bundled characters should load");

        assert!(!characters.is_empty());
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
    fn classifies_qte_as_utf8_huanhe() {
        assert_eq!(
            classify_attack_type(None, "GE_Player_Test_QTE_Damage", Some("GA_Test_QTE")),
            "环合"
        );
    }

    #[test]
    fn loads_chinese_damage_names_from_wooden_assets() {
        let names = load_wooden_damage_names(Path::new(WOODEN_DAMAGE_DESCRIPTIONS_PATH)).unwrap();

        assert_eq!(
            names
                .get("GE_Player_Sagiri_QTE1_Damage")
                .map(String::as_str),
            Some("早雾环合")
        );
        assert_eq!(
            names
                .get("GE_Player_Nanally_Melee1_Damage")
                .map(String::as_str),
            Some("娜娜莉普攻")
        );
        assert_eq!(
            classify_attack_type_from_description("早雾大招1").as_deref(),
            Some("Q技能")
        );
    }

    #[test]
    fn loads_compact_wooden_damage_names() {
        let path = write_temp_json(
            "compact_wooden_names.json",
            r#"{"names":{"GE_Player_Sagiri_QTE1_Damage":"早雾环合"}}"#,
        );

        let names = load_wooden_damage_names(&path).unwrap();

        assert_eq!(
            names
                .get("GE_Player_Sagiri_QTE1_Damage")
                .map(String::as_str),
            Some("早雾环合")
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
        let handle = [
            0x21, 0xf0, 0x4e, 0x92, 0x89, 0x95, 0x33, 0x4f, 0x8c, 0x0b, 0xbc, 0xaa, 0x0e, 0xe1,
            0x6f, 0xe7,
        ];
        let mut payload = [0_u8; BOSS_HP_PREFIX_LENGTH + 4];
        payload[..8].copy_from_slice(&BOSS_HP_PREFIX_HEAD);
        payload[8..24].copy_from_slice(&handle);
        payload[BOSS_HP_PREFIX_LENGTH..].copy_from_slice(&1_927_891_f32.to_le_bytes());

        let updates = parse_boss_hp_updates(&payload);

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].target_handle, handle);
        assert_eq!(updates[0].current_hp, 1_927_891.0);
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
        assert_eq!(hits[0].direction, "incoming");
        assert_eq!(hits[0].char_id, 1001);
        assert!(hits[0].target_id.is_none());
        assert!(hits[0].target_name.is_none());
        assert!(hits[0].target_context.is_empty());
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
        assert_eq!(hits[0].direction, "outgoing");
        assert_eq!(hits[0].char_id, 1001);
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
        assert_eq!(hits[0].direction, "outgoing");
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
        assert_eq!(hits[0].direction, "outgoing");
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
        assert_eq!(hits[0].direction, "outgoing");
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
        assert_eq!(hits[0].direction, "outgoing");
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
