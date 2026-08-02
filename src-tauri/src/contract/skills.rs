use serde::Serialize;

use nte_dps_tool::{
    core::skills::{
        SkillsDiagnosticsProjection, SkillsProjection, SkillsRowProjection, SkillsScope,
        skill_label_translation_key,
    },
    storage::{ability_names, i18n},
};

pub(crate) const SKILLS_CONTRACT_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkillsSnapshot {
    pub contract_version: u32,
    pub generation: String,
    pub scope: &'static str,
    pub has_data: bool,
    pub total_damage: f64,
    pub total_hits: String,
    pub characters: Vec<SkillsCharacterSnapshot>,
    pub rows: Vec<SkillsRowSnapshot>,
    pub diagnostics: SkillsDiagnosticsSnapshot,
}

impl SkillsSnapshot {
    pub(crate) fn from_projection(
        projection: SkillsProjection,
        generation: u64,
        scope: SkillsScope,
    ) -> Self {
        Self {
            contract_version: SKILLS_CONTRACT_VERSION,
            generation: generation.to_string(),
            scope: scope_code(scope),
            has_data: !projection.rows.is_empty(),
            total_damage: projection.total_damage,
            total_hits: projection.total_hits.to_string(),
            characters: projection
                .characters
                .into_iter()
                .map(|character| SkillsCharacterSnapshot {
                    id: character.id,
                    name: character.name,
                    color: character.color,
                    damage: character.damage,
                    entries: character.entries.min(u32::MAX as usize) as u32,
                })
                .collect(),
            rows: projection.rows.into_iter().map(localized_row).collect(),
            diagnostics: SkillsDiagnosticsSnapshot::from(projection.diagnostics),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkillsCharacterSnapshot {
    pub id: u32,
    pub name: String,
    pub color: String,
    pub damage: f64,
    pub entries: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkillsRowSnapshot {
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
    pub hits: String,
    pub damage: f64,
}

fn localized_row(row: SkillsRowProjection) -> SkillsRowSnapshot {
    let name = if row.follow_up {
        localized_label(&row.name)
    } else {
        localized_skill_name(&row)
    };
    SkillsRowSnapshot {
        id: row.id,
        character_id: row.character_id,
        character_name: row.character_name,
        name,
        category: localized_label(&row.category),
        ability_name: row.ability_name,
        damage_name: row.damage_name,
        gameplay_effect_index: row.gameplay_effect_index,
        gameplay_effect_name: row.gameplay_effect_name,
        follow_up: row.follow_up,
        hits: row.hits.to_string(),
        damage: row.damage,
    }
}

fn localized_skill_name(row: &SkillsRowProjection) -> String {
    let ability_name = row
        .ability_name
        .as_deref()
        .or_else(|| row.name.starts_with("GA_").then_some(row.name.as_str()));
    let gameplay_effect_name = row
        .gameplay_effect_name
        .as_deref()
        .or_else(|| row.name.starts_with("GE_").then_some(row.name.as_str()));
    gameplay_effect_name
        .and_then(ability_names::resolve_damage_name)
        .or_else(|| ability_name.and_then(ability_names::resolve_ability_name))
        .or_else(|| row.damage_name.clone())
        .unwrap_or_else(|| localized_label(&row.name))
}

fn localized_label(label: &str) -> String {
    if let Some(key) = skill_label_translation_key(label) {
        return i18n::t(key);
    }
    if let Some(reaction) = label.strip_prefix("环合·")
        && let Some(key) = skill_label_translation_key(reaction)
    {
        return format!("{} · {}", i18n::t("Esper Cycle"), i18n::t(key));
    }
    label.to_owned()
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkillsDiagnosticsSnapshot {
    pub unknown_character_count: String,
    pub unknown_character_hits: String,
    pub unknown_direction_hits: String,
    pub unknown_direction_damage: f64,
    pub unmapped_skill_rows: String,
    pub unmapped_skill_hits: String,
    pub unmapped_skill_damage: f64,
    pub unmapped_gameplay_effects: Vec<SkillsUnknownEffectSnapshot>,
}

impl From<SkillsDiagnosticsProjection> for SkillsDiagnosticsSnapshot {
    fn from(value: SkillsDiagnosticsProjection) -> Self {
        Self {
            unknown_character_count: value.unknown_character_count.to_string(),
            unknown_character_hits: value.unknown_character_hits.to_string(),
            unknown_direction_hits: value.unknown_direction_hits.to_string(),
            unknown_direction_damage: value.unknown_direction_damage,
            unmapped_skill_rows: value.unmapped_skill_rows.to_string(),
            unmapped_skill_hits: value.unmapped_skill_hits.to_string(),
            unmapped_skill_damage: value.unmapped_skill_damage,
            unmapped_gameplay_effects: value
                .unmapped_gameplay_effects
                .into_iter()
                .map(|effect| SkillsUnknownEffectSnapshot {
                    index: effect.index,
                    hits: effect.hits.to_string(),
                    damage: effect.damage,
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkillsUnknownEffectSnapshot {
    pub index: u32,
    pub hits: String,
    pub damage: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", content = "payload", rename_all = "camelCase")]
pub(crate) enum SkillsEvent {
    Snapshot(SkillsSnapshot),
}

pub(crate) const fn scope_code(scope: SkillsScope) -> &'static str {
    match scope {
        SkillsScope::Whole => "all",
        SkillsScope::First => "upper",
        SkillsScope::Second => "lower",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nte_dps_tool::core::skills::{SkillsCharacterProjection, SkillsUnknownEffectProjection};

    #[test]
    fn snapshot_serializes_unsafe_counters_as_decimal_strings() {
        let projection = SkillsProjection {
            total_damage: 42.0,
            total_hits: u64::MAX,
            characters: vec![SkillsCharacterProjection {
                id: 1,
                name: "Character".to_owned(),
                color: "#123abc".to_owned(),
                damage: 42.0,
                entries: 1,
            }],
            diagnostics: SkillsDiagnosticsProjection {
                unknown_character_hits: u64::MAX,
                unmapped_gameplay_effects: vec![SkillsUnknownEffectProjection {
                    index: 7,
                    hits: u64::MAX,
                    damage: 42.0,
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let snapshot =
            SkillsSnapshot::from_projection(projection, 9_007_199_254_740_992, SkillsScope::Whole);
        let value = serde_json::to_value(snapshot).expect("skills snapshot serializes");

        assert_eq!(value["generation"], "9007199254740992");
        assert_eq!(value["totalHits"], u64::MAX.to_string());
        assert_eq!(
            value["diagnostics"]["unknownCharacterHits"],
            u64::MAX.to_string()
        );
        assert_eq!(
            value["diagnostics"]["unmappedGameplayEffects"][0]["hits"],
            u64::MAX.to_string()
        );
    }

    #[test]
    fn category_localization_uses_stable_keys_and_preserves_unknown_labels() {
        assert_eq!(localized_label("E技能"), i18n::t("Skill"));
        assert_eq!(
            localized_label("环合·创生"),
            format!("{} · {}", i18n::t("Esper Cycle"), i18n::t("Blossom"))
        );
        assert_eq!(localized_label("Custom"), "Custom");
    }
}
