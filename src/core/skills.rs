//! Frontend-neutral projection for the Console skills page.

use std::collections::HashMap;

use crate::{
    engine::model::{
        AbyssHalf, CharacterInfo, CombatState, SkillBreakdownRow, summarize_skill_breakdown,
    },
    storage::i18n::Language,
};

use super::timeline::{character_color, character_name, finite_non_negative};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SkillsScope {
    #[default]
    Whole,
    First,
    Second,
}

#[derive(Clone, Copy, Debug)]
pub struct SkillsProjectionOptions {
    pub scope: SkillsScope,
    pub language: Language,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkillsProjection {
    pub total_damage: f64,
    pub total_hits: u64,
    pub characters: Vec<SkillsCharacterProjection>,
    pub rows: Vec<SkillsRowProjection>,
    pub diagnostics: SkillsDiagnosticsProjection,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkillsCharacterProjection {
    pub id: u32,
    pub name: String,
    pub color: String,
    pub damage: f64,
    pub entries: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkillsRowProjection {
    pub id: String,
    pub character_id: u32,
    pub character_name: String,
    pub name: String,
    pub category: String,
    pub ability_name: Option<String>,
    pub damage_name: Option<String>,
    pub gameplay_effect_index: Option<u32>,
    pub gameplay_effect_name: Option<String>,
    pub follow_up: bool,
    pub hits: u64,
    pub damage: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkillsDiagnosticsProjection {
    pub unknown_character_count: usize,
    pub unknown_character_hits: u64,
    pub unknown_direction_hits: u64,
    pub unknown_direction_damage: f64,
    pub unmapped_skill_rows: usize,
    pub unmapped_skill_hits: u64,
    pub unmapped_skill_damage: f64,
    pub unmapped_gameplay_effects: Vec<SkillsUnknownEffectProjection>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkillsUnknownEffectProjection {
    pub index: u32,
    pub hits: u64,
    pub damage: f64,
}

pub fn project_skills(
    state: &CombatState,
    characters: &HashMap<u32, CharacterInfo>,
    options: SkillsProjectionOptions,
) -> SkillsProjection {
    let hits = match options.scope {
        SkillsScope::Whole => &state.hits,
        SkillsScope::First => &state.abyss.half(AbyssHalf::First).hits,
        SkillsScope::Second => &state.abyss.half(AbyssHalf::Second).hits,
    };
    let breakdown = summarize_skill_breakdown(hits, None);
    let mut summaries = HashMap::<u32, (String, f64, usize)>::new();
    for row in &breakdown.rows {
        let entry = summaries
            .entry(row.char_id)
            .or_insert_with(|| (row.char_name.clone(), 0.0, 0));
        if !row.char_name.is_empty() {
            entry.0.clone_from(&row.char_name);
        }
        entry.1 += finite_non_negative(row.damage);
        entry.2 += 1;
    }
    let mut projected_characters = summaries
        .into_iter()
        .map(
            |(id, (fallback_name, damage, entries))| SkillsCharacterProjection {
                id,
                name: character_name(id, &fallback_name, characters, options.language),
                color: character_color(id, characters),
                damage,
                entries,
            },
        )
        .collect::<Vec<_>>();
    projected_characters.sort_by(|left, right| {
        right
            .damage
            .total_cmp(&left.damage)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });

    let rows = breakdown
        .rows
        .iter()
        .map(|row| project_row(row, characters, options.language))
        .collect();
    let unknown = breakdown.unknown;
    SkillsProjection {
        total_damage: finite_non_negative(breakdown.total_damage),
        total_hits: breakdown.total_hits,
        characters: projected_characters,
        rows,
        diagnostics: SkillsDiagnosticsProjection {
            unknown_character_count: unknown.unknown_character_count,
            unknown_character_hits: unknown.unknown_character_hits,
            unknown_direction_hits: unknown.unknown_direction_hits,
            unknown_direction_damage: finite_non_negative(unknown.unknown_direction_damage),
            unmapped_skill_rows: unknown.unmapped_skill_rows,
            unmapped_skill_hits: unknown.unmapped_skill_hits,
            unmapped_skill_damage: finite_non_negative(unknown.unmapped_skill_damage),
            unmapped_gameplay_effects: unknown
                .unmapped_gameplay_effects
                .into_iter()
                .map(|effect| SkillsUnknownEffectProjection {
                    index: effect.index,
                    hits: effect.hits,
                    damage: finite_non_negative(effect.damage),
                })
                .collect(),
        },
    }
}

fn project_row(
    row: &SkillBreakdownRow,
    characters: &HashMap<u32, CharacterInfo>,
    language: Language,
) -> SkillsRowProjection {
    SkillsRowProjection {
        id: stable_row_id(row),
        character_id: row.char_id,
        character_name: character_name(row.char_id, &row.char_name, characters, language),
        name: row.name.clone(),
        category: row.category.clone(),
        ability_name: row.ability_name.clone(),
        damage_name: row.damage_name.clone(),
        gameplay_effect_index: row.gameplay_effect_index,
        gameplay_effect_name: row.gameplay_effect_name.clone(),
        follow_up: row.is_follow_up,
        hits: row.hits,
        damage: finite_non_negative(row.damage),
    }
}

fn stable_row_id(row: &SkillBreakdownRow) -> String {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    for part in [
        row.char_id.to_string(),
        row.name.clone(),
        row.category.clone(),
        row.ability_name.clone().unwrap_or_default(),
        row.gameplay_effect_index
            .map(|value| value.to_string())
            .unwrap_or_default(),
        row.gameplay_effect_name.clone().unwrap_or_default(),
        row.is_follow_up.to_string(),
    ] {
        for byte in part.bytes().chain(std::iter::once(0xff)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }
    format!("skill-{hash:016x}")
}

/// Stable English translation key for parser-produced attack categories and
/// reaction labels. UI frontends translate the key at their own boundary.
pub fn skill_label_translation_key(label: &str) -> Option<&'static str> {
    match label {
        "创生" => Some("Blossom"),
        "创生花" => Some("Blossom Damage"),
        "覆纹" => Some("Hexed"),
        "覆纹追加攻击" => Some("Hexed Follow-up Attack"),
        "延滞" => Some("Remora"),
        "黯星" => Some("Nova"),
        "浊燃" => Some("Scorch"),
        "浸染" => Some("Stain"),
        "盈蓄" => Some("Charge"),
        "失谐" => Some("Discord"),
        "环合" => Some("Esper Cycle"),
        "环合伤害" => Some("Reaction Damage"),
        "倾陷伤害" => Some("Break Damage"),
        "普攻" => Some("Basic Attack"),
        "E技能" => Some("Skill"),
        "Q技能" => Some("Ultimate"),
        "闪避反击" => Some("Parry Attack"),
        "格挡反击" => Some("Block Counter"),
        "载具伤害" => Some("Vehicle Damage"),
        "深渊场地Buff" => Some("Abyss Field Buff"),
        "HP同步伤害" => Some("HP Sync Damage"),
        "Passive Damage" => Some("Passive Damage"),
        "Special Damage" => Some("Special Damage"),
        "Awakening Damage" => Some("Awakening Damage"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::model::{Hit, HitCharacterSource, HitDirection};

    fn hit(damage: f64) -> Hit {
        Hit {
            timestamp: 1.0,
            char_id: 7,
            char_name: "Fallback".to_owned(),
            damage,
            char_known: true,
            byte_offset: 0,
            bit_shift: 0,
            char_source: HitCharacterSource::Packet,
            direction: HitDirection::Outgoing,
            target_hp_before: 1_000.0,
            target_hp_after: 1_000.0 - damage,
            target_max_hp: 1_000.0,
            target_hp_percent: 50.0,
            target_id: None,
            target_name: None,
            target_name_en: None,
            target_name_ja: None,
            target_monster_id: None,
            target_context: Vec::new(),
            gameplay_effect_index: Some(17),
            gameplay_effect_name: Some("GE_Test".to_owned()),
            ability_name: Some("GA_Test".to_owned()),
            damage_name: Some("Test Damage".to_owned()),
            damage_component: None,
            attack_type: Some("E技能".to_owned()),
            damage_attribute: None,
            follow_up_damage: 0.0,
            follow_up_timestamp: None,
            follow_up_damage_name: None,
            follow_up_attack_type: None,
            follow_up_damage_attribute: None,
        }
    }

    #[test]
    fn projection_aggregates_rows_and_localizes_characters() {
        let mut state = CombatState::default();
        state.push_hit(hit(125.0));
        let characters = HashMap::from([(
            7,
            CharacterInfo {
                name_zh: "角色七".to_owned(),
                name_en: "Character Seven".to_owned(),
                color: Some("#123abc".to_owned()),
                avatar: None,
                attribute: None,
            },
        )]);

        let projection = project_skills(
            &state,
            &characters,
            SkillsProjectionOptions {
                scope: SkillsScope::Whole,
                language: Language::SimplifiedChinese,
            },
        );

        assert_eq!(projection.total_damage, 125.0);
        assert_eq!(projection.total_hits, 1);
        assert_eq!(projection.characters[0].name, "角色七");
        assert_eq!(projection.characters[0].color, "#123abc");
        assert_eq!(projection.rows[0].character_name, "角色七");
        assert!(projection.rows[0].id.starts_with("skill-"));
    }

    #[test]
    fn stable_row_identity_ignores_changing_damage_and_hits() {
        let mut first = summarize_skill_breakdown([&hit(10.0)], None)
            .rows
            .pop()
            .expect("skill row");
        let first_id = stable_row_id(&first);
        first.damage = 999.0;
        first.hits = 42;
        assert_eq!(stable_row_id(&first), first_id);
    }

    #[test]
    fn translation_keys_cover_skill_categories() {
        assert_eq!(skill_label_translation_key("E技能"), Some("Skill"));
        assert_eq!(
            skill_label_translation_key("创生花"),
            Some("Blossom Damage")
        );
        assert_eq!(skill_label_translation_key("自定义招式"), None);
    }
}
