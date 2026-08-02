use std::collections::{HashMap, VecDeque};

use serde::Serialize;

use nte_dps_tool::{
    core::combat_details::CombatDetailFilter,
    engine::model::{
        CharacterInfo, CharacterStats, CombatState, DamageAttributionSummary, Hit, HitDirection,
        HitDirectionSummary, PartyCombatState, is_qte_follow_up_damage_type,
        is_unbalance_damage_hit, summarize_hit_directions,
    },
    storage::{config::DpsTimeMode, i18n::Language},
};

use crate::state::{AppState, MainDpsDetailRequest};

pub(crate) const MAIN_DPS_DETAIL_CONTRACT_VERSION: u32 = 2;
pub(crate) const MAIN_DPS_DETAIL_PAGE_LIMIT: usize = 250;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsDetailSnapshot {
    pub contract_version: u32,
    pub generation: String,
    pub kind: &'static str,
    pub abyss_half: Option<&'static str>,
    pub character_id: Option<u32>,
    pub character_name: Option<String>,
    pub character_color: Option<String>,
    pub filter: &'static str,
    pub qte_type: Option<String>,
    pub skill_filter: Option<String>,
    pub metrics: MainDpsDetailMetrics,
    pub direction: MainDpsDirectionSummary,
    pub hit_types: Vec<MainDpsFilterSummary>,
    pub attribution: MainDpsAttributionSummary,
    pub qte_summaries: Vec<MainDpsQteSummary>,
    pub skills: Vec<MainDpsSkillSummary>,
    pub total_hits: usize,
    pub total_damage: f64,
    pub offset: usize,
    pub rows: Vec<MainDpsHitSnapshot>,
}

impl MainDpsDetailSnapshot {
    pub(crate) fn from_state(state: &AppState, offset: usize, limit: usize) -> Self {
        let request = state.main_dps_detail_request();
        let resources = state.live_capture_resources();
        let config = state.ui_config_snapshot();
        let language = config.language;
        let subtract_time_stop = matches!(config.dps_time_mode, DpsTimeMode::TimeStopAdjusted);
        let (combat, selected_half) = state.main_dps_detail_state();
        let source = selected_half
            .map(|half| DetailSource::Party(combat.abyss.half(half)))
            .unwrap_or(DetailSource::Combat(&combat));
        let base_hits = source
            .hits()
            .iter()
            .filter(|hit| request.character_id.is_none_or(|id| hit.char_id == id))
            .collect::<Vec<_>>();
        let matching = base_hits
            .iter()
            .copied()
            .filter(|hit| request.matches(hit))
            .collect::<Vec<_>>();
        let total_hits = matching.len();
        let total_damage = matching.iter().map(|hit| hit.total_damage()).sum();
        let rows = matching
            .into_iter()
            .skip(offset)
            .take(limit.clamp(1, MAIN_DPS_DETAIL_PAGE_LIMIT))
            .enumerate()
            .map(|(index, hit)| {
                MainDpsHitSnapshot::from_hit(hit, offset + index, &resources.characters, language)
            })
            .collect();
        let character = request
            .character_id
            .and_then(|character_id| resources.characters.get(&character_id));
        let character_name = request.character_id.map(|character_id| {
            localized_character_name(
                character,
                language,
                &source
                    .stats()
                    .get(&character_id)
                    .map(|row| row.name.as_str())
                    .unwrap_or_else(|| "-"),
            )
        });
        let metrics = detail_metrics(
            source,
            request.character_id,
            config.separate_reaction_damage,
            subtract_time_stop,
        );
        let direction = summarize_hit_directions(base_hits.iter().copied()).into();
        let attribution = MainDpsAttributionSummary::new(
            source.damage_attribution_summary(),
            config.separate_reaction_damage,
        );
        let skills = request
            .character_id
            .map(|_| skill_summaries(&base_hits, metrics.total_output))
            .unwrap_or_default();
        let qte_summaries = qte_summaries(&base_hits, metrics.total_output);
        let qte_type = match &request.filter {
            CombatDetailFilter::QteType(value) => Some(value.clone()),
            _ => None,
        };

        Self {
            contract_version: MAIN_DPS_DETAIL_CONTRACT_VERSION,
            generation: state.next_sequence().to_string(),
            kind: if request.character_id.is_some() {
                "character"
            } else {
                "team"
            },
            abyss_half: selected_half.map(|half| match half {
                nte_dps_tool::engine::model::AbyssHalf::First => "first",
                nte_dps_tool::engine::model::AbyssHalf::Second => "second",
            }),
            character_id: request.character_id,
            character_name,
            character_color: character.and_then(|value| value.color.clone()),
            filter: filter_id(&request.filter),
            qte_type,
            skill_filter: request.skill_filter,
            metrics,
            direction,
            hit_types: hit_type_summaries(&base_hits),
            attribution,
            qte_summaries,
            skills,
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
            && self
                .skill_filter
                .as_ref()
                .is_none_or(|filter| hit_skill_name(hit) == *filter)
    }
}

#[derive(Clone, Copy)]
enum DetailSource<'a> {
    Combat(&'a CombatState),
    Party(&'a PartyCombatState),
}

impl<'a> DetailSource<'a> {
    fn hits(self) -> &'a VecDeque<Hit> {
        match self {
            Self::Combat(value) => &value.hits,
            Self::Party(value) => &value.hits,
        }
    }

    fn stats(self) -> &'a HashMap<u32, CharacterStats> {
        match self {
            Self::Combat(value) => &value.stats,
            Self::Party(value) => &value.stats,
        }
    }

    fn total_damage(self) -> f64 {
        match self {
            Self::Combat(value) => value.total_damage,
            Self::Party(value) => value.total_damage,
        }
    }

    fn total_damage_taken(self) -> f64 {
        match self {
            Self::Combat(value) => value.total_damage_taken,
            Self::Party(value) => value.total_damage_taken,
        }
    }

    fn duration(self, subtract_time_stop: bool) -> f64 {
        match self {
            Self::Combat(value) => value.duration_with_time_stop(subtract_time_stop),
            Self::Party(value) => value.duration_with_time_stop(subtract_time_stop),
        }
    }

    fn character_duration(self, row: &CharacterStats, subtract_time_stop: bool) -> f64 {
        match self {
            Self::Combat(value) => value.character_duration_with_time_stop(row, subtract_time_stop),
            Self::Party(value) => value.character_duration_with_time_stop(row, subtract_time_stop),
        }
    }

    fn damage_attribution_summary(self) -> DamageAttributionSummary {
        match self {
            Self::Combat(value) => value.damage_attribution_summary(),
            Self::Party(value) => value.damage_attribution_summary(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsDetailMetrics {
    pub total_output: f64,
    pub dps: f64,
    pub output_count: u64,
    pub incoming_count: u64,
    pub total_damage_taken: f64,
    pub duration_seconds: f64,
}

fn detail_metrics(
    source: DetailSource<'_>,
    character_id: Option<u32>,
    separate_reaction_damage: bool,
    subtract_time_stop: bool,
) -> MainDpsDetailMetrics {
    if let Some(character_id) = character_id {
        let row = source
            .stats()
            .get(&character_id)
            .cloned()
            .unwrap_or_default()
            .for_reaction_damage_policy(separate_reaction_damage);
        let duration = source.character_duration(&row, subtract_time_stop);
        return MainDpsDetailMetrics {
            total_output: row.damage,
            dps: row.damage / duration.max(1.0),
            output_count: row.hits,
            incoming_count: row.hits_taken,
            total_damage_taken: row.damage_taken,
            duration_seconds: duration,
        };
    }
    let duration = source.duration(subtract_time_stop);
    MainDpsDetailMetrics {
        total_output: source.total_damage(),
        dps: source.total_damage() / duration.max(1.0),
        output_count: source.stats().values().map(|row| row.hits).sum(),
        incoming_count: source.stats().values().map(|row| row.hits_taken).sum(),
        total_damage_taken: source.total_damage_taken(),
        duration_seconds: duration,
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsDirectionSummary {
    pub confirmed_output: f64,
    pub confirmed_hits: u64,
    pub candidate_output: f64,
    pub candidate_hits: u64,
    pub incoming_output: f64,
    pub incoming_hits: u64,
    pub candidate_share_percent: f64,
}

impl From<HitDirectionSummary> for MainDpsDirectionSummary {
    fn from(value: HitDirectionSummary) -> Self {
        Self {
            confirmed_output: value.outgoing_damage,
            confirmed_hits: value.outgoing_hits,
            candidate_output: value.unknown_damage,
            candidate_hits: value.unknown_hits,
            incoming_output: value.incoming_damage,
            incoming_hits: value.incoming_hits,
            candidate_share_percent: value.unknown_share(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsFilterSummary {
    pub id: &'static str,
    pub hits: usize,
    pub damage: f64,
}

fn hit_type_summaries(hits: &[&Hit]) -> Vec<MainDpsFilterSummary> {
    [
        ("all", CombatDetailFilter::All),
        ("outgoing", CombatDetailFilter::Outgoing),
        ("incoming", CombatDetailFilter::Incoming),
    ]
    .into_iter()
    .map(|(id, filter)| MainDpsFilterSummary {
        id,
        hits: hits.iter().filter(|hit| filter.matches(hit)).count(),
        damage: hits
            .iter()
            .filter(|hit| filter.matches(hit))
            .map(|hit| hit.total_damage())
            .sum(),
    })
    .collect()
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsAttributionSummary {
    pub total_damage: f64,
    pub character_damage: f64,
    pub character_filter: &'static str,
    pub reaction_damage: f64,
    pub shared_damage: f64,
    pub unattributed_damage: f64,
    pub separate_reaction_damage: bool,
}

impl MainDpsAttributionSummary {
    fn new(value: DamageAttributionSummary, separate_reaction_damage: bool) -> Self {
        Self {
            total_damage: value.total_damage,
            character_damage: value.character_damage(separate_reaction_damage),
            character_filter: if separate_reaction_damage {
                "characterDirect"
            } else {
                "characterAttributed"
            },
            reaction_damage: value.character_reaction_damage,
            shared_damage: value.shared_damage,
            unattributed_damage: value.unattributed_damage,
            separate_reaction_damage,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsQteSummary {
    pub attack_type: String,
    pub hits: u64,
    pub damage: f64,
    pub share_percent: f64,
}

fn qte_summaries(hits: &[&Hit], total_damage: f64) -> Vec<MainDpsQteSummary> {
    let mut summaries = HashMap::<String, (u64, f64)>::new();
    for hit in hits
        .iter()
        .copied()
        .filter(|hit| !hit.direction.is_incoming())
    {
        if let Some(attack_type) = hit.attack_type.as_deref()
            && (is_qte_follow_up_damage_type(attack_type) || is_unbalance_damage_hit(hit))
        {
            let row = summaries.entry(attack_type.to_owned()).or_default();
            row.0 += 1;
            row.1 += hit.damage;
        }
        if hit.follow_up_damage > 0.0
            && let Some(attack_type) = hit.follow_up_attack_type.as_deref()
            && is_qte_follow_up_damage_type(attack_type)
        {
            let row = summaries.entry(attack_type.to_owned()).or_default();
            row.0 += 1;
            row.1 += hit.follow_up_damage;
        }
    }
    let mut rows = summaries
        .into_iter()
        .map(|(attack_type, (hits, damage))| MainDpsQteSummary {
            attack_type,
            hits,
            damage,
            share_percent: percent(damage, total_damage),
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| right.damage.total_cmp(&left.damage));
    rows
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsSkillSummary {
    pub id: String,
    pub name: String,
    pub category: String,
    pub hits: u64,
    pub damage: f64,
    pub share_percent: f64,
}

fn skill_summaries(hits: &[&Hit], total_damage: f64) -> Vec<MainDpsSkillSummary> {
    let mut summaries = HashMap::<String, (String, u64, f64)>::new();
    for hit in hits
        .iter()
        .copied()
        .filter(|hit| !hit.direction.is_incoming())
    {
        let name = hit_skill_name(hit);
        let row = summaries.entry(name.clone()).or_insert_with(|| {
            (
                hit.attack_type
                    .clone()
                    .unwrap_or_else(|| "Uncategorized".to_owned()),
                0,
                0.0,
            )
        });
        row.1 += 1;
        row.2 += hit.total_damage();
    }
    let mut rows = summaries
        .into_iter()
        .map(|(id, (category, hits, damage))| MainDpsSkillSummary {
            name: id.clone(),
            id,
            category,
            hits,
            damage,
            share_percent: percent(damage, total_damage),
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| right.damage.total_cmp(&left.damage));
    rows
}

fn percent(value: f64, total: f64) -> f64 {
    if total > 0.0 {
        value / total * 100.0
    } else {
        0.0
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
    pub primary_damage: f64,
    pub follow_up_damage: f64,
    pub skill_id: String,
    pub skill: String,
    pub damage_type: String,
    pub target: String,
    pub target_hp_after: f64,
    pub target_max_hp: f64,
    pub target_hp_percent: f64,
}

impl MainDpsHitSnapshot {
    fn from_hit(
        hit: &Hit,
        index: usize,
        characters: &HashMap<u32, CharacterInfo>,
        language: Language,
    ) -> Self {
        let skill = hit_skill_name(hit);
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
            primary_damage: hit.damage,
            follow_up_damage: hit.follow_up_damage,
            skill_id: skill.clone(),
            skill,
            damage_type: hit
                .attack_type
                .as_deref()
                .or(hit.damage_attribute.as_deref())
                .unwrap_or("-")
                .to_owned(),
            target: localized_target_name(hit, language)
                .unwrap_or("-")
                .to_owned(),
            target_hp_after: hit.target_hp_after,
            target_max_hp: hit.target_max_hp,
            target_hp_percent: hit.target_hp_percent,
        }
    }
}

fn hit_skill_name(hit: &Hit) -> String {
    hit.damage_component
        .as_deref()
        .or(hit.ability_name.as_deref())
        .or(hit.gameplay_effect_name.as_deref())
        .or(hit.damage_name.as_deref())
        .or(hit.attack_type.as_deref())
        .unwrap_or("Unmapped Skill")
        .to_owned()
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
        assert_eq!(
            filter_id(&CombatDetailFilter::QteType("创生花".to_owned())),
            "qteType"
        );
    }

    #[test]
    fn candidate_direction_summary_keeps_unknown_share() {
        let summary = MainDpsDirectionSummary::from(HitDirectionSummary {
            outgoing_damage: 75.0,
            outgoing_hits: 3,
            unknown_damage: 25.0,
            unknown_hits: 1,
            incoming_damage: 5.0,
            incoming_hits: 1,
        });
        assert_eq!(summary.confirmed_hits, 3);
        assert_eq!(summary.candidate_share_percent, 25.0);
    }

    #[test]
    fn empty_state_serializes_the_complete_detail_contract() {
        let snapshot = MainDpsDetailSnapshot::from_state(&AppState::default(), 0, 200);
        let value = serde_json::to_value(snapshot).expect("detail snapshot serializes");

        assert_eq!(value["contractVersion"], MAIN_DPS_DETAIL_CONTRACT_VERSION);
        assert_eq!(value["metrics"]["totalOutput"], 0.0);
        assert_eq!(value["direction"]["candidateHits"], 0);
        assert_eq!(value["hitTypes"].as_array().map(Vec::len), Some(3));
        assert_eq!(value["rows"].as_array().map(Vec::len), Some(0));
    }
}
