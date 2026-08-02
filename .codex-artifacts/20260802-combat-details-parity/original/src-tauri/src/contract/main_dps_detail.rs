use serde::Serialize;

use nte_dps_tool::{
    core::combat_details::CombatDetailFilter,
    engine::model::{CharacterInfo, Hit, HitDirection},
    storage::i18n::Language,
};

use crate::state::{AppState, MainDpsDetailRequest};

pub(crate) const MAIN_DPS_DETAIL_CONTRACT_VERSION: u32 = 1;
pub(crate) const MAIN_DPS_DETAIL_PAGE_LIMIT: usize = 250;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsDetailSnapshot {
    pub contract_version: u32,
    pub generation: String,
    pub kind: &'static str,
    pub character_id: Option<u32>,
    pub character_name: Option<String>,
    pub filter: &'static str,
    pub total_hits: usize,
    pub total_damage: f64,
    pub offset: usize,
    pub rows: Vec<MainDpsHitSnapshot>,
}

impl MainDpsDetailSnapshot {
    pub(crate) fn from_state(state: &AppState, offset: usize, limit: usize) -> Self {
        let request = state.main_dps_detail_request();
        let resources = state.live_capture_resources();
        let language = state.ui_config_snapshot().language;
        let (combat, selected_half) = state.main_dps_detail_state();
        let hits = selected_half
            .map(|half| &combat.abyss.half(half).hits)
            .unwrap_or(&combat.hits);
        let matching = hits.iter().filter(|hit| request.matches(hit));
        let total_hits = matching.clone().count();
        let total_damage = matching.clone().map(Hit::total_damage).sum();
        let rows = matching
            .skip(offset)
            .take(limit.clamp(1, MAIN_DPS_DETAIL_PAGE_LIMIT))
            .enumerate()
            .map(|(index, hit)| {
                MainDpsHitSnapshot::from_hit(hit, offset + index, &resources.characters, language)
            })
            .collect();
        let character_name = request.character_id.map(|character_id| {
            localized_character_name(
                resources.characters.get(&character_id),
                language,
                &character_id.to_string(),
            )
        });

        Self {
            contract_version: MAIN_DPS_DETAIL_CONTRACT_VERSION,
            generation: state.next_sequence().to_string(),
            kind: if request.character_id.is_some() {
                "character"
            } else {
                "team"
            },
            character_id: request.character_id,
            character_name,
            filter: filter_id(&request.filter),
            total_hits,
            total_damage,
            offset,
            rows,
        }
    }
}

impl MainDpsDetailRequest {
    fn matches(&self, hit: &Hit) -> bool {
        self.character_id
            .is_none_or(|character_id| hit.char_id == character_id)
            && self.filter.matches(hit)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsHitSnapshot {
    pub id: String,
    pub timestamp: f64,
    pub character_id: u32,
    pub character_name: String,
    pub direction: &'static str,
    pub damage: f64,
    pub skill: String,
    pub target: String,
}

impl MainDpsHitSnapshot {
    fn from_hit(
        hit: &Hit,
        index: usize,
        characters: &std::collections::HashMap<u32, CharacterInfo>,
        language: Language,
    ) -> Self {
        Self {
            id: format!("{}:{index}", hit.timestamp.to_bits()),
            timestamp: hit.timestamp,
            character_id: hit.char_id,
            character_name: localized_character_name(
                characters.get(&hit.char_id),
                language,
                &hit.char_name,
            ),
            direction: match hit.direction {
                HitDirection::Outgoing => "outgoing",
                HitDirection::Incoming => "incoming",
                HitDirection::Unknown => "unknown",
            },
            damage: hit.total_damage(),
            skill: hit
                .ability_name
                .as_deref()
                .or(hit.damage_name.as_deref())
                .or(hit.gameplay_effect_name.as_deref())
                .unwrap_or("-")
                .to_owned(),
            target: localized_target_name(hit, language)
                .unwrap_or("-")
                .to_owned(),
        }
    }
}

fn localized_character_name(
    info: Option<&CharacterInfo>,
    language: Language,
    fallback: &str,
) -> String {
    let candidate = info.map(|value| match language {
        Language::SimplifiedChinese => value.name_zh.trim(),
        Language::English | Language::Japanese => value.name_en.trim(),
    });
    candidate
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

fn localized_target_name(hit: &Hit, language: Language) -> Option<&str> {
    let value = match language {
        Language::SimplifiedChinese => hit.target_name.as_deref(),
        Language::English => hit.target_name_en.as_deref().or(hit.target_name.as_deref()),
        Language::Japanese => hit
            .target_name_ja
            .as_deref()
            .or(hit.target_name_en.as_deref())
            .or(hit.target_name.as_deref()),
    }?;
    (!value.trim().is_empty()).then_some(value)
}

pub(crate) fn filter_id(filter: &CombatDetailFilter) -> &'static str {
    match filter {
        CombatDetailFilter::All => "all",
        CombatDetailFilter::Outgoing => "outgoing",
        CombatDetailFilter::Incoming => "incoming",
        CombatDetailFilter::CharacterAttributed => "characterAttributed",
        CombatDetailFilter::CharacterDirect => "characterDirect",
        CombatDetailFilter::ReactionDamage => "reactionDamage",
        CombatDetailFilter::SharedMechanics => "sharedMechanics",
        CombatDetailFilter::Unattributed => "unattributed",
        CombatDetailFilter::QteType(_) => "qteType",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_filters_have_stable_frontend_ids() {
        assert_eq!(filter_id(&CombatDetailFilter::All), "all");
        assert_eq!(
            filter_id(&CombatDetailFilter::CharacterDirect),
            "characterDirect"
        );
        assert_eq!(
            filter_id(&CombatDetailFilter::SharedMechanics),
            "sharedMechanics"
        );
    }
}
