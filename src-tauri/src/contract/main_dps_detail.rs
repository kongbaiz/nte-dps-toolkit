use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use nte_dps_tool::{
    core::{
        combat_details::{
            CombatDetailFilter, damage_digit_key_for_hit, follow_up_damage_digit_key_for_hit,
            reaction_text_key_for_hit,
        },
        live_capture::LiveCapturePhase,
    },
    engine::model::{
        CharacterInfo, CharacterStats, CombatState, DamageAttributionSummary, Hit, HitDirection,
        HitDirectionSummary, PartyCombatState, is_qte_follow_up_damage_type,
        is_unbalance_damage_hit, summarize_hit_directions,
    },
    storage::{
        ability_names,
        config::{DpsTimeMode, HitDetailColumnsConfig},
        i18n::{self, Language},
    },
};

use crate::state::{AppState, MainDpsDetailKind, MainDpsDetailRequest};

pub(crate) const MAIN_DPS_DETAIL_CONTRACT_VERSION: u32 = 3;
pub(crate) const MAIN_DPS_DETAIL_DEFAULT_LIMIT: usize = 200;
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
    pub columns: MainDpsDetailColumns,
    pub actions: MainDpsDetailActions,
    pub metrics: MainDpsDetailMetrics,
    pub direction: MainDpsDirectionSummary,
    pub hit_types: Vec<MainDpsFilterSummary>,
    pub attribution: MainDpsAttributionSummary,
    pub qte_summaries: Vec<MainDpsQteSummary>,
    pub skills: Vec<MainDpsSkillSummary>,
    pub total_hits: usize,
    pub total_damage: f64,
    pub max_row_damage: f64,
    pub offset: usize,
    pub rows: Vec<MainDpsHitSnapshot>,
}

impl MainDpsDetailSnapshot {
    pub(crate) fn from_state(
        state: &AppState,
        kind: MainDpsDetailKind,
        offset: usize,
        limit: usize,
    ) -> Self {
        let request = state.main_dps_detail_request(kind);
        let resources = state.live_capture_resources();
        let config = state.ui_config_snapshot();
        let language = config.language;
        let subtract_time_stop = matches!(config.dps_time_mode, DpsTimeMode::TimeStopAdjusted);
        let generation = state.next_sequence().to_string();
        let actions = MainDpsDetailActions::from_state(state);
        state.with_main_dps_detail_state(|combat, selected_half| {
            let source = selected_half
                .map(|half| DetailSource::Party(combat.abyss.half(half)))
                .unwrap_or(DetailSource::Combat(combat));
            let base_hits = source
                .hits()
                .iter()
                .filter(|hit| request.character_id.is_none_or(|id| hit.char_id == id))
                .collect::<Vec<_>>();
            let page_limit = limit.clamp(1, MAIN_DPS_DETAIL_PAGE_LIMIT);
            let mut total_hits = 0_usize;
            let mut total_damage = 0.0_f64;
            let mut max_row_damage = 1.0_f64;
            let mut rows = Vec::with_capacity(page_limit);
            for hit in base_hits.iter().copied().filter(|hit| request.matches(hit)) {
                let row_index = total_hits;
                total_hits += 1;
                let damage = hit.total_damage();
                total_damage += damage;
                max_row_damage = max_row_damage.max(damage);
                if row_index >= offset && rows.len() < page_limit {
                    rows.push(MainDpsHitSnapshot::from_hit(
                        hit,
                        row_index,
                        &resources.characters,
                        language,
                    ));
                }
            }
            let character = request
                .character_id
                .and_then(|character_id| resources.characters.get(&character_id));
            let character_name = request.character_id.map(|character_id| {
                localized_character_name(
                    character,
                    language,
                    source
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
            let mut direction: MainDpsDirectionSummary =
                summarize_hit_directions(base_hits.iter().copied()).into();
            direction.confirmed_hits = metrics.output_count;
            let hit_types = hit_type_summaries(&metrics);
            let attribution = MainDpsAttributionSummary::new(
                source.damage_attribution_summary(),
                config.separate_reaction_damage,
            );
            let skills = request
                .character_id
                .map(|_| skill_summaries(&base_hits, metrics.total_output, language))
                .unwrap_or_default();
            let qte_summaries = qte_summaries(&base_hits, metrics.total_output);
            let qte_type = match &request.filter {
                CombatDetailFilter::QteType(value) => Some(value.clone()),
                _ => None,
            };

            Self {
                contract_version: MAIN_DPS_DETAIL_CONTRACT_VERSION,
                generation,
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
                columns: config.hit_detail_columns.into(),
                actions,
                metrics,
                direction,
                hit_types,
                attribution,
                qte_summaries,
                skills,
                total_hits,
                total_damage,
                max_row_damage,
                offset,
                rows,
            }
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MainDpsDetailColumns {
    pub show_time: bool,
    pub show_character: bool,
    pub show_type: bool,
    pub show_damage: bool,
    pub show_target: bool,
    pub time_width: u16,
    pub character_width: u16,
    pub type_width: u16,
    pub damage_width: u16,
    pub target_width: u16,
}

impl From<HitDetailColumnsConfig> for MainDpsDetailColumns {
    fn from(value: HitDetailColumnsConfig) -> Self {
        Self {
            show_time: value.show_time,
            show_character: value.show_character,
            show_type: value.show_type,
            show_damage: value.show_damage,
            show_target: value.show_target_hp,
            time_width: value.time_width,
            character_width: value.character_width,
            type_width: value.type_width,
            damage_width: value.damage_width,
            target_width: value.target_hp_width,
        }
    }
}

impl From<MainDpsDetailColumns> for HitDetailColumnsConfig {
    fn from(value: MainDpsDetailColumns) -> Self {
        Self {
            show_time: value.show_time,
            show_character: value.show_character,
            show_type: value.show_type,
            show_damage: value.show_damage,
            show_target_hp: value.show_target,
            time_width: value.time_width,
            character_width: value.character_width,
            type_width: value.type_width,
            damage_width: value.damage_width,
            target_hp_width: value.target_width,
        }
        .sanitized()
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsDetailActions {
    pub can_start_capture: bool,
    pub can_import_replay: bool,
}

impl MainDpsDetailActions {
    fn from_state(state: &AppState) -> Self {
        let capture_active = matches!(
            state.capture_phase(),
            LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
        );
        let replay_running = state.replay_running();
        Self {
            can_start_capture: !capture_active && !replay_running,
            can_import_replay: !capture_active && !replay_running,
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

fn hit_type_summaries(metrics: &MainDpsDetailMetrics) -> Vec<MainDpsFilterSummary> {
    [
        (
            "all",
            metrics.output_count.saturating_add(metrics.incoming_count),
            metrics.total_output + metrics.total_damage_taken,
        ),
        ("outgoing", metrics.output_count, metrics.total_output),
        (
            "incoming",
            metrics.incoming_count,
            metrics.total_damage_taken,
        ),
    ]
    .into_iter()
    .map(|(id, hits, damage)| MainDpsFilterSummary {
        id,
        hits: usize::try_from(hits).unwrap_or(usize::MAX),
        damage,
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

#[derive(Clone, Debug)]
struct SkillSummaryAccumulator {
    name: String,
    category: String,
    hits: u64,
    damage: f64,
}

fn skill_summaries(
    hits: &[&Hit],
    total_damage: f64,
    language: Language,
) -> Vec<MainDpsSkillSummary> {
    let mut summaries = HashMap::<String, SkillSummaryAccumulator>::new();
    for hit in hits
        .iter()
        .copied()
        .filter(|hit| !hit.direction.is_incoming())
    {
        let id = hit_skill_name(hit);
        let row = summaries
            .entry(id)
            .or_insert_with(|| SkillSummaryAccumulator {
                name: skill_summary_display_name(hit, language),
                category: hit
                    .attack_type
                    .as_deref()
                    .map(|value| translate_attack_type(value, language))
                    .unwrap_or_else(|| i18n::t_for(language, "Uncategorized")),
                hits: 0,
                damage: 0.0,
            });
        row.hits += 1;
        row.damage += hit.total_damage();
    }
    let mut rows = summaries
        .into_iter()
        .map(|(id, summary)| MainDpsSkillSummary {
            id,
            name: summary.name,
            category: summary.category,
            hits: summary.hits,
            damage: summary.damage,
            share_percent: percent(summary.damage, total_damage),
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| right.damage.total_cmp(&left.damage));
    rows
}

fn skill_summary_display_name(hit: &Hit, language: Language) -> String {
    let stable_name = hit_skill_name(hit);
    let resource_name = hit
        .gameplay_effect_name
        .as_deref()
        .and_then(ability_names::resolve_damage_name)
        .or_else(|| {
            hit.ability_name
                .as_deref()
                .and_then(ability_names::resolve_ability_name)
        });
    if let Some(name) = resource_name {
        return name;
    }

    let legacy_name = hit
        .damage_component
        .as_deref()
        .or(hit.damage_name.as_deref())
        .filter(|value| !value.trim().is_empty());
    if let Some(name) = legacy_name.filter(|value| !is_technical_skill_name(value)) {
        return name.to_owned();
    }
    hit.attack_type
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| translate_attack_type(value, language))
        .or_else(|| legacy_name.map(str::to_owned))
        .unwrap_or(stable_name)
}

fn is_technical_skill_name(value: &str) -> bool {
    value.starts_with("GA_")
        || value.starts_with("GE_")
        || value.starts_with("Buff_")
        || value.contains('_')
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
    pub type_label: String,
    pub reaction_text_key: Option<u8>,
    pub damage_digit_key: Option<String>,
    pub follow_up_damage_digit_key: Option<String>,
    pub target: String,
    pub target_monster_id: Option<String>,
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
            type_label: hit_type_display_text(hit, language),
            reaction_text_key: reaction_text_key_for_hit(hit),
            damage_digit_key: damage_digit_key_for_hit(hit, characters).map(str::to_owned),
            follow_up_damage_digit_key: follow_up_damage_digit_key_for_hit(hit).map(str::to_owned),
            target: localized_target_name(hit, language)
                .unwrap_or("-")
                .to_owned(),
            target_monster_id: hit.target_monster_id.clone(),
            target_hp_after: hit.target_hp_after,
            target_max_hp: hit.target_max_hp,
            target_hp_percent: hit.target_hp_percent,
        }
    }
}

fn hit_type_display_text(hit: &Hit, language: Language) -> String {
    match hit.direction {
        HitDirection::Incoming => return i18n::t_for(language, "Incoming"),
        HitDirection::Unknown => return i18n::t_for(language, "Candidate Output"),
        HitDirection::Outgoing => {}
    }
    let attack_type = hit
        .attack_type
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|value| translate_attack_type(value, language));
    let stable_name = hit_skill_name(hit);
    let name = (hit.ability_name.is_some()
        || hit.gameplay_effect_name.is_some()
        || hit.damage_component.is_some()
        || hit.damage_name.is_some())
    .then(|| {
        hit.gameplay_effect_name
            .as_deref()
            .and_then(ability_names::resolve_damage_name)
            .or_else(|| {
                hit.ability_name
                    .as_deref()
                    .and_then(ability_names::resolve_ability_name)
            })
            .or_else(|| {
                hit.damage_component
                    .as_deref()
                    .or(hit.damage_name.as_deref())
                    .map(str::to_owned)
            })
            .unwrap_or(stable_name)
    });
    match (attack_type.as_deref(), name.as_deref()) {
        (Some(kind), Some(name)) if hit.attack_type.as_deref() != Some(name) && kind != name => {
            format!("{kind}·{name}")
        }
        (Some(kind), _) => kind.to_owned(),
        (None, Some(name)) => name.to_owned(),
        (None, None) => i18n::t_for(language, "Unmapped Skill"),
    }
}

fn translate_attack_type(value: &str, language: Language) -> String {
    if let Some(key) = nte_dps_tool::core::skills::skill_label_translation_key(value) {
        return i18n::t_for(language, key);
    }
    if let Some(reaction) = value.strip_prefix("环合·")
        && let Some(key) = nte_dps_tool::core::skills::skill_label_translation_key(reaction)
    {
        return format!(
            "{} · {}",
            i18n::t_for(language, "Esper Cycle"),
            i18n::t_for(language, key)
        );
    }
    value.to_owned()
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
    use nte_dps_tool::engine::model::{HitCharacterSource, HitDirection};

    fn skill_hit(
        damage: f64,
        ability_name: Option<&str>,
        damage_name: Option<&str>,
        attack_type: &str,
    ) -> Hit {
        Hit {
            timestamp: 1.0,
            char_id: 1010,
            char_name: "娜娜莉".to_owned(),
            char_known: true,
            damage,
            byte_offset: 0,
            bit_shift: 0,
            char_source: HitCharacterSource::Packet,
            direction: HitDirection::Outgoing,
            target_hp_before: 1000.0,
            target_hp_after: 1000.0 - damage,
            target_max_hp: 1000.0,
            target_hp_percent: (1000.0 - damage) / 10.0,
            target_id: None,
            target_name: None,
            target_name_en: None,
            target_name_ja: None,
            target_monster_id: None,
            target_context: Vec::new(),
            gameplay_effect_index: None,
            gameplay_effect_name: None,
            ability_name: ability_name.map(str::to_owned),
            damage_name: damage_name.map(str::to_owned),
            damage_component: None,
            attack_type: Some(attack_type.to_owned()),
            damage_attribute: Some("灵".to_owned()),
            follow_up_damage: 0.0,
            follow_up_timestamp: None,
            follow_up_damage_name: None,
            follow_up_attack_type: None,
            follow_up_damage_attribute: None,
        }
    }

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
        let snapshot = MainDpsDetailSnapshot::from_state(
            &AppState::default(),
            MainDpsDetailKind::Team,
            0,
            200,
        );
        let value = serde_json::to_value(snapshot).expect("detail snapshot serializes");

        assert_eq!(value["contractVersion"], MAIN_DPS_DETAIL_CONTRACT_VERSION);
        assert_eq!(value["metrics"]["totalOutput"], 0.0);
        assert_eq!(value["direction"]["candidateHits"], 0);
        assert_eq!(value["hitTypes"].as_array().map(Vec::len), Some(3));
        assert_eq!(value["columns"]["showTime"], true);
        assert_eq!(value["actions"]["canStartCapture"], true);
        assert_eq!(value["maxRowDamage"], 1.0);
        assert_eq!(value["rows"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn fifty_thousand_hits_keep_totals_correct_and_return_only_the_requested_page() {
        let mut combat = CombatState::default();
        for index in 0..50_000 {
            let mut hit = skill_hit(1.0, Some("GA_Fixture"), Some("Fixture"), "Skill");
            hit.timestamp = index as f64;
            combat.push_hit(hit);
        }
        let state = AppState::default();
        state.restore_live_state_for_test(
            combat,
            nte_dps_tool::engine::model::CaptureQualitySource::Live,
        );

        let snapshot = MainDpsDetailSnapshot::from_state(
            &state,
            MainDpsDetailKind::Team,
            49_990,
            MAIN_DPS_DETAIL_PAGE_LIMIT,
        );

        assert_eq!(snapshot.total_hits, 50_000);
        assert_eq!(snapshot.total_damage, 50_000.0);
        assert_eq!(snapshot.rows.len(), 10);
        assert_eq!(snapshot.rows[0].timestamp, 49_990.0);
    }

    #[test]
    fn filter_counts_use_the_authoritative_metric_counts() {
        let metrics = MainDpsDetailMetrics {
            total_output: 2_300_409.0,
            dps: 72_741.0,
            output_count: 509,
            incoming_count: 3,
            total_damage_taken: 9_383.0,
            duration_seconds: 31.6,
        };
        let summaries = hit_type_summaries(&metrics);
        assert_eq!(summaries[0].hits, 512);
        assert_eq!(summaries[1].hits, 509);
        assert_eq!(summaries[2].hits, 3);
    }

    #[test]
    fn skill_summaries_keep_stable_filter_ids_but_never_display_technical_names() {
        let (_, warning) = ability_names::init(Language::SimplifiedChinese);
        assert_eq!(warning, None);
        let ultimate = skill_hit(100.0, Some("GA_Nanally_UltraSkill"), None, "Q技能");
        let mut awakening = skill_hit(
            20.0,
            None,
            Some("Awakening Follow-up Attack"),
            "Awakening Damage",
        );
        awakening.gameplay_effect_name = Some("GE_Nanally010_Lv3_Damage".to_owned());
        awakening.damage_component = Some("Awakening Follow-up Attack".to_owned());
        let break_damage = skill_hit(10.0, None, Some("Buff_Tenacity_damage"), "倾陷伤害");
        let hits = [&ultimate, &awakening, &break_damage];

        let summaries = skill_summaries(&hits, 130.0, Language::SimplifiedChinese);
        let ultimate = summaries
            .iter()
            .find(|summary| summary.id == "GA_Nanally_UltraSkill")
            .expect("ultimate summary");
        assert_eq!(ultimate.name, "柯林斯·终极术");
        let awakening = summaries
            .iter()
            .find(|summary| summary.id == "Awakening Follow-up Attack")
            .expect("awakening follow-up summary");
        assert_eq!(awakening.name, "觉醒追加攻击");
        let break_damage = summaries
            .iter()
            .find(|summary| summary.id == "Buff_Tenacity_damage")
            .expect("break damage summary");
        assert_eq!(break_damage.name, "倾陷伤害");
    }
}
