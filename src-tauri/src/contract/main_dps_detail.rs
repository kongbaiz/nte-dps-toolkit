use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use nte_dps_tool::{
    core::{
        CoreError,
        combat_details::{
            CombatDetailFilter, damage_digit_key_for_hit, follow_up_damage_digit_key_for_hit,
            reaction_text_key_for_hit,
        },
        live_capture::LiveCapturePhase,
    },
    engine::model::{
        CharacterInfo, CharacterStats, CombatState, DamageAttributionSummary, Hit, HitDirection,
        HitDirectionSummary, MAX_INDEXED_DETAIL_KEY_BYTES, PartyCombatState,
    },
    storage::{
        ability_names,
        config::HitDetailColumnsConfig,
        i18n::{self, Language},
    },
};

use crate::state::{AppState, MainDpsDetailKind};

pub(crate) const MAIN_DPS_DETAIL_CONTRACT_VERSION: u32 = 6;
pub(crate) const MAIN_DPS_DETAIL_DEFAULT_LIMIT: usize = 200;
pub(crate) const MAIN_DPS_DETAIL_PAGE_LIMIT: usize = 250;
pub(crate) const MAIN_DPS_DETAIL_QTE_LIMIT: usize = 32;
pub(crate) const MAIN_DPS_DETAIL_SKILL_LIMIT: usize = 250;
pub(crate) const MAIN_DPS_DETAIL_MAX_TEXT_BYTES: usize = MAX_INDEXED_DETAIL_KEY_BYTES;
/// Dynamic strings may expand by up to six bytes per source byte when JSON
/// escapes control characters. Keeping the complete projected text payload at
/// 512 KiB leaves ample room below the shared 16 MiB stream envelope for all
/// bounded rows, numeric metadata, and framing.
pub(crate) const MAIN_DPS_DETAIL_MAX_PROJECTED_TEXT_BYTES: usize = 512 * 1024;

#[derive(Default)]
struct MainDpsDetailTextBudget {
    projected_bytes: usize,
    truncated: bool,
}

impl MainDpsDetailTextBudget {
    fn text(&mut self, value: String, fallback: &str) -> String {
        let value = if value.len() <= MAIN_DPS_DETAIL_MAX_TEXT_BYTES {
            value
        } else {
            self.truncated = true;
            fallback.to_owned()
        };
        if self
            .projected_bytes
            .checked_add(value.len())
            .is_some_and(|next| next <= MAIN_DPS_DETAIL_MAX_PROJECTED_TEXT_BYTES)
        {
            self.projected_bytes += value.len();
            return value;
        }
        self.truncated = true;
        if self
            .projected_bytes
            .checked_add(fallback.len())
            .is_some_and(|next| next <= MAIN_DPS_DETAIL_MAX_PROJECTED_TEXT_BYTES)
        {
            self.projected_bytes += fallback.len();
            fallback.to_owned()
        } else {
            String::new()
        }
    }

    fn optional(&mut self, value: Option<String>) -> Option<String> {
        value.and_then(|value| {
            let bounded = self.text(value, "");
            (!bounded.is_empty()).then_some(bounded)
        })
    }
}

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
    pub qte_summary_total_count: usize,
    pub qte_summaries_truncated: bool,
    pub skills: Vec<MainDpsSkillSummary>,
    pub skill_total_count: usize,
    pub skills_truncated: bool,
    /// True when at least one external/resource-derived string was replaced or
    /// omitted to preserve the per-field or cumulative stream byte budget.
    pub text_truncated: bool,
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
    ) -> Result<Self, CoreError> {
        let request = state.main_dps_detail_request(kind);
        let cache_revision = state.main_dps_stream_revision()?;
        if let Some(snapshot) =
            state.main_dps_detail_cache_get(cache_revision, kind, &request, offset, limit)
        {
            return Ok((*snapshot).clone());
        }
        let resources = state.live_capture_resources();
        let config = state.ui_config_snapshot();
        let language = config.language;
        let subtract_time_stop = matches!(
            state.main_presented_combat_clock_health()?,
            nte_dps_tool::engine::model::CombatClockRuntimeHealth::Available
                | nte_dps_tool::engine::model::CombatClockRuntimeHealth::Recorded
        ) && matches!(
            config.dps_time_mode,
            nte_dps_tool::storage::config::DpsTimeMode::TimeStopAdjusted
        );
        let generation = state.next_sequence().to_string();
        let actions = MainDpsDetailActions::from_state(state)?;
        let snapshot = state.with_main_dps_detail_state(|combat, selected_half| {
            let mut text_budget = MainDpsDetailTextBudget::default();
            let source = selected_half
                .map(|half| DetailSource::Party(combat.abyss.half(half)))
                .unwrap_or(DetailSource::Combat(combat));
            let page_limit = limit.clamp(1, MAIN_DPS_DETAIL_PAGE_LIMIT);
            let indexed = source.indexed_combat_details(
                request.character_id,
                &request.filter.indexed(),
                request.skill_filter.as_deref(),
                offset,
                page_limit,
            );
            let total_hits = indexed.total_hits;
            let total_damage = indexed.total_damage;
            let max_row_damage = indexed.max_row_damage;
            let generic_target_labels = GenericTargetLabels::from_source(source, language);
            let rows = indexed
                .rows
                .into_iter()
                .enumerate()
                .map(|(page_index, (_, hit))| {
                    MainDpsHitSnapshot::from_hit(
                        hit,
                        offset.saturating_add(page_index),
                        &resources.characters,
                        language,
                        &generic_target_labels,
                        &mut text_budget,
                    )
                })
                .collect::<Vec<_>>();
            let character = request
                .character_id
                .and_then(|character_id| resources.characters.get(&character_id));
            let character_name = request.character_id.map(|character_id| {
                text_budget.text(
                    localized_character_name(
                        character,
                        language,
                        source
                            .stats()
                            .get(&character_id)
                            .map(|row| row.name.as_str())
                            .unwrap_or_else(|| "-"),
                    ),
                    "-",
                )
            });
            let metrics = detail_metrics(
                source,
                request.character_id,
                config.separate_reaction_damage,
                subtract_time_stop,
            );
            let direction: MainDpsDirectionSummary = indexed.directions.into();
            let hit_types = hit_type_summaries(&metrics);
            let attribution = MainDpsAttributionSummary::new(
                source.damage_attribution_summary(),
                config.separate_reaction_damage,
            );
            let qte_summary_total_count = indexed.qte_summaries.len();
            let qte_summaries_truncated = qte_summary_total_count > MAIN_DPS_DETAIL_QTE_LIMIT;
            let qte_summaries = indexed
                .qte_summaries
                .into_iter()
                .take(MAIN_DPS_DETAIL_QTE_LIMIT)
                .map(|summary| MainDpsQteSummary {
                    attack_type: text_budget.text(summary.attack_type, "-"),
                    hits: summary.hits,
                    damage: summary.damage,
                    share_percent: percent(summary.damage, metrics.total_output),
                })
                .collect::<Vec<_>>();
            let skill_total_count = if request.character_id.is_some() {
                indexed.skill_summaries.len()
            } else {
                0
            };
            let skills_truncated = skill_total_count > MAIN_DPS_DETAIL_SKILL_LIMIT;
            let skills = if request.character_id.is_some() {
                indexed
                    .skill_summaries
                    .into_iter()
                    .take(MAIN_DPS_DETAIL_SKILL_LIMIT)
                    .filter_map(|summary| {
                        let representative = source.hits().get(summary.representative_position)?;
                        Some(MainDpsSkillSummary {
                            id: text_budget.text(summary.id, "-"),
                            name: text_budget
                                .text(skill_summary_display_name(representative, language), "-"),
                            category: text_budget.text(
                                representative
                                    .attack_type
                                    .as_deref()
                                    .map(|value| translate_attack_type(value, language))
                                    .unwrap_or_else(|| i18n::t_for(language, "Uncategorized")),
                                "-",
                            ),
                            hits: summary.hits,
                            damage: summary.damage,
                            share_percent: percent(summary.damage, metrics.total_output),
                        })
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let qte_type = match &request.filter {
                CombatDetailFilter::QteType(value) => Some(text_budget.text(value.clone(), "-")),
                _ => None,
            };
            let skill_filter = text_budget.optional(request.skill_filter.clone());
            let character_color =
                text_budget.optional(character.and_then(|value| value.color.clone()));

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
                character_color,
                filter: filter_id(&request.filter),
                qte_type,
                skill_filter,
                columns: config.hit_detail_columns.into(),
                actions,
                metrics,
                direction,
                hit_types,
                attribution,
                qte_summaries,
                qte_summary_total_count,
                qte_summaries_truncated,
                skills,
                skill_total_count,
                skills_truncated,
                text_truncated: text_budget.truncated,
                total_hits,
                total_damage,
                max_row_damage,
                offset,
                rows,
            }
        })?;
        state.main_dps_detail_cache_store(
            cache_revision,
            kind,
            &request,
            offset,
            limit,
            std::sync::Arc::new(snapshot.clone()),
        );
        Ok(snapshot)
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
    fn from_state(state: &AppState) -> Result<Self, CoreError> {
        let capture_active = matches!(
            state.capture_phase(),
            LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
        );
        let replay_running = state.replay_running()?;
        Ok(Self {
            can_start_capture: !capture_active && !replay_running,
            can_import_replay: !capture_active && !replay_running,
        })
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

    fn indexed_combat_details(
        self,
        character_id: Option<u32>,
        filter: &nte_dps_tool::engine::model::IndexedCombatDetailFilter,
        skill: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> nte_dps_tool::engine::model::IndexedCombatDetailPage<'a> {
        match self {
            Self::Combat(value) => {
                value.indexed_combat_details(character_id, filter, skill, offset, limit)
            }
            Self::Party(value) => {
                value.indexed_combat_details(character_id, filter, skill, offset, limit)
            }
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

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
struct SkillSummaryAccumulator<'a> {
    representative: &'a Hit,
    hits: u64,
    damage: f64,
}

#[cfg(test)]
fn accumulate_skill_summary<'a>(
    summaries: &mut HashMap<&'a str, SkillSummaryAccumulator<'a>>,
    hit: &'a Hit,
) {
    let id = hit_skill_name_ref(hit);
    let row = summaries.entry(id).or_insert(SkillSummaryAccumulator {
        representative: hit,
        hits: 0,
        damage: 0.0,
    });
    row.hits += 1;
    row.damage += hit.total_damage();
}

#[cfg(test)]
fn skill_summaries_from_accumulators(
    summaries: HashMap<&str, SkillSummaryAccumulator<'_>>,
    total_damage: f64,
    language: Language,
) -> Vec<MainDpsSkillSummary> {
    let mut rows = summaries
        .into_iter()
        .map(|(id, summary)| MainDpsSkillSummary {
            id: id.to_owned(),
            name: skill_summary_display_name(summary.representative, language),
            category: summary
                .representative
                .attack_type
                .as_deref()
                .map(|value| translate_attack_type(value, language))
                .unwrap_or_else(|| i18n::t_for(language, "Uncategorized")),
            hits: summary.hits,
            damage: summary.damage,
            share_percent: percent(summary.damage, total_damage),
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| right.damage.total_cmp(&left.damage));
    rows
}

#[cfg(test)]
fn skill_summaries<'a>(
    hits: impl IntoIterator<Item = &'a Hit>,
    total_damage: f64,
    language: Language,
) -> Vec<MainDpsSkillSummary> {
    let mut summaries = HashMap::<&'a str, SkillSummaryAccumulator<'a>>::new();
    for hit in hits {
        if !hit.direction.is_incoming() {
            accumulate_skill_summary(&mut summaries, hit);
        }
    }
    skill_summaries_from_accumulators(summaries, total_damage, language)
}

fn skill_summary_display_name(hit: &Hit, language: Language) -> String {
    if is_target_hp_residual(hit) {
        return i18n::t_for(language, "Target HP Residual");
    }
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

fn is_target_hp_residual(hit: &Hit) -> bool {
    hit.damage_name.as_deref() == Some("Target HP Residual")
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
    pub overkill_damage: f64,
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

#[derive(Default)]
struct GenericTargetLabels {
    by_id: HashMap<String, String>,
}

impl GenericTargetLabels {
    fn from_source(source: DetailSource<'_>, language: Language) -> Self {
        Self::from_hits(source.hits().iter(), language)
    }

    fn from_hits<'a>(hits: impl IntoIterator<Item = &'a Hit>, language: Language) -> Self {
        let mut candidates = HashSet::<String>::new();
        for hit in hits {
            if localized_target_name(hit, language).is_some() {
                continue;
            }
            let Some(target_id) = hit
                .target_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            candidates.insert(target_id.to_owned());
        }

        let mut ordered = candidates.into_iter().collect::<Vec<_>>();
        ordered.sort();
        let mut by_id = HashMap::with_capacity(ordered.len());
        for (index, target_id) in ordered.into_iter().enumerate() {
            let prefix = i18n::t_for(language, "Enemy");
            by_id.insert(target_id, format!("{prefix}{}", index + 1));
        }
        Self { by_id }
    }

    fn for_hit(&self, hit: &Hit) -> Option<&str> {
        hit.target_id
            .as_deref()
            .and_then(|target_id| self.by_id.get(target_id))
            .map(String::as_str)
    }
}

impl MainDpsHitSnapshot {
    fn from_hit(
        hit: &Hit,
        index: usize,
        characters: &HashMap<u32, CharacterInfo>,
        language: Language,
        generic_target_labels: &GenericTargetLabels,
        text_budget: &mut MainDpsDetailTextBudget,
    ) -> Self {
        let skill = if is_target_hp_residual(hit) {
            i18n::t_for(language, "Target HP Residual")
        } else {
            hit_skill_name(hit)
        };
        let skill_id = text_budget.text(skill, "-");
        let skill = text_budget.text(skill_id.clone(), "-");
        Self {
            id: text_budget.text(format!("{}:{index}", hit.timestamp.to_bits()), "-"),
            timestamp: hit.timestamp,
            character_id: hit.char_id,
            character_name: text_budget.text(
                localized_character_name(characters.get(&hit.char_id), language, &hit.char_name),
                "-",
            ),
            direction: match hit.direction {
                HitDirection::Outgoing => "outgoing",
                HitDirection::Incoming => "incoming",
                HitDirection::Unknown => "unknown",
            },
            damage: hit.total_damage(),
            primary_damage: hit.damage,
            follow_up_damage: hit.follow_up_damage,
            overkill_damage: hit.overkill_damage(),
            skill_id,
            skill,
            damage_type: text_budget.text(
                hit.attack_type
                    .as_deref()
                    .or(hit.damage_attribute.as_deref())
                    .unwrap_or("-")
                    .to_owned(),
                "-",
            ),
            type_label: text_budget.text(hit_type_display_text(hit, language), "-"),
            reaction_text_key: reaction_text_key_for_hit(hit),
            damage_digit_key: text_budget
                .optional(damage_digit_key_for_hit(hit, characters).map(str::to_owned)),
            follow_up_damage_digit_key: text_budget
                .optional(follow_up_damage_digit_key_for_hit(hit).map(str::to_owned)),
            target: text_budget.text(
                if is_target_hp_residual(hit) {
                    i18n::t_for(language, "Completed Targets")
                } else {
                    localized_target_name(hit, language)
                        .or_else(|| generic_target_labels.for_hit(hit))
                        .unwrap_or("-")
                        .to_owned()
                },
                "-",
            ),
            target_monster_id: text_budget.optional(hit.target_monster_id.clone()),
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
    if is_target_hp_residual(hit) {
        return i18n::t_for(language, "Target HP Residual");
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
    hit_skill_name_ref(hit).to_owned()
}

fn hit_skill_name_ref(hit: &Hit) -> &str {
    hit.damage_component
        .as_deref()
        .or(hit.ability_name.as_deref())
        .or(hit.gameplay_effect_name.as_deref())
        .or(hit.damage_name.as_deref())
        .or(hit.attack_type.as_deref())
        .unwrap_or("Unmapped Skill")
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
    use crate::{
        channels::stream_runtime::serialize_stream_events,
        contract::stream::MAX_STREAM_DELIVERY_BYTES, state::MainDpsDetailRequest,
    };
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
            reconciled_overkill_damage: None,
            wire_event: None,
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
    fn unresolved_unique_targets_receive_stable_generic_labels() {
        let mut first_minion = skill_hit(100.0, Some("GA_Test"), None, "Skill");
        first_minion.target_id = Some("enemy-instance:0001".to_owned());
        first_minion.target_max_hp = 808_898.0;
        first_minion.target_monster_id = Some("mon_015".to_owned());

        let mut boss = skill_hit(100.0, Some("GA_Test"), None, "Skill");
        boss.timestamp = 2.0;
        boss.target_id = Some("enemy-instance:0002".to_owned());
        boss.target_max_hp = 2_628_918.0;
        boss.target_monster_id = Some("Boss_015".to_owned());

        let mut second_minion = skill_hit(100.0, Some("GA_Test"), None, "Skill");
        second_minion.timestamp = 3.0;
        second_minion.target_id = Some("enemy-instance:0003".to_owned());
        second_minion.target_max_hp = 808_898.0;
        second_minion.target_context = vec!["enemy_config_id=mon_016_BP".to_owned()];

        let mut unknown = skill_hit(100.0, Some("GA_Test"), None, "Skill");
        unknown.timestamp = 4.0;
        unknown.target_id = Some("enemy-instance:0004".to_owned());

        let hits = VecDeque::from([
            first_minion.clone(),
            boss.clone(),
            second_minion.clone(),
            unknown.clone(),
            first_minion.clone(),
        ]);
        let labels = GenericTargetLabels::from_hits(hits.iter(), Language::SimplifiedChinese);
        let characters = HashMap::new();
        let mut text_budget = MainDpsDetailTextBudget::default();

        let first = MainDpsHitSnapshot::from_hit(
            &first_minion,
            0,
            &characters,
            Language::SimplifiedChinese,
            &labels,
            &mut text_budget,
        );
        let boss = MainDpsHitSnapshot::from_hit(
            &boss,
            1,
            &characters,
            Language::SimplifiedChinese,
            &labels,
            &mut text_budget,
        );
        let second = MainDpsHitSnapshot::from_hit(
            &second_minion,
            2,
            &characters,
            Language::SimplifiedChinese,
            &labels,
            &mut text_budget,
        );
        let repeated = MainDpsHitSnapshot::from_hit(
            &first_minion,
            4,
            &characters,
            Language::SimplifiedChinese,
            &labels,
            &mut text_budget,
        );
        let unknown = MainDpsHitSnapshot::from_hit(
            &unknown,
            3,
            &characters,
            Language::SimplifiedChinese,
            &labels,
            &mut text_budget,
        );

        assert_eq!(first.target, "敌人1");
        assert_eq!(boss.target, "敌人2");
        assert_eq!(second.target, "敌人3");
        assert_eq!(unknown.target, "敌人4");
        assert_eq!(repeated.target, "敌人1");
    }

    #[test]
    fn generic_target_labels_are_stable_across_real_pages_and_filters() {
        let mut first = skill_hit(100.0, Some("GA_Test"), None, "Skill");
        first.target_id = Some("enemy-instance:0001".to_owned());
        let mut second = skill_hit(100.0, Some("GA_Test"), None, "Skill");
        second.timestamp = 2.0;
        second.target_id = Some("enemy-instance:0002".to_owned());
        second.direction = HitDirection::Incoming;
        let mut combat = CombatState::default();
        combat.push_hit(first);
        combat.push_hit(second);
        let source = DetailSource::Combat(&combat);
        let labels = GenericTargetLabels::from_source(source, Language::SimplifiedChinese);
        let page = source.indexed_combat_details(
            None,
            &nte_dps_tool::engine::model::IndexedCombatDetailFilter::All,
            None,
            1,
            1,
        );
        let filtered = source.indexed_combat_details(
            None,
            &nte_dps_tool::engine::model::IndexedCombatDetailFilter::Incoming,
            None,
            0,
            1,
        );

        assert_eq!(labels.for_hit(page.rows[0].1), Some("敌人2"));
        assert_eq!(labels.for_hit(filtered.rows[0].1), Some("敌人2"));
    }

    #[test]
    fn target_kind_metadata_does_not_change_generic_enemy_labels() {
        let mut minion = skill_hit(100.0, Some("GA_Test"), None, "Skill");
        minion.target_id = Some("enemy-instance:0001".to_owned());
        minion.target_context = vec!["target_kind=minion".to_owned()];
        let mut boss = skill_hit(100.0, Some("GA_Test"), None, "Skill");
        boss.target_id = Some("enemy-instance:0002".to_owned());
        boss.target_context = vec!["target_kind=boss".to_owned()];

        let labels = GenericTargetLabels::from_hits([&minion, &boss], Language::SimplifiedChinese);

        assert_eq!(labels.for_hit(&minion), Some("敌人1"));
        assert_eq!(labels.for_hit(&boss), Some("敌人2"));
    }

    #[test]
    fn target_hp_residual_is_localized_as_explicit_unattributed_damage() {
        let mut hit = skill_hit(100.0, None, None, "Skill");
        hit.damage_name = Some("Target HP Residual".to_owned());

        assert_eq!(
            skill_summary_display_name(&hit, Language::SimplifiedChinese),
            "未归属目标生命差值"
        );
        assert_eq!(
            hit_type_display_text(&hit, Language::Japanese),
            "未帰属の対象HP差分"
        );

        let snapshot = MainDpsHitSnapshot::from_hit(
            &hit,
            0,
            &HashMap::new(),
            Language::SimplifiedChinese,
            &GenericTargetLabels::default(),
            &mut MainDpsDetailTextBudget::default(),
        );
        assert_eq!(snapshot.target, "本阶段已击败目标");
    }

    #[test]
    fn hit_snapshot_records_primary_overkill_damage() {
        let mut hit = skill_hit(1_500.0, Some("GA_Test"), None, "Skill");
        hit.target_hp_before = 1_000.0;
        hit.target_max_hp = 10_000.0;
        hit.follow_up_damage = 250.0;

        let snapshot = MainDpsHitSnapshot::from_hit(
            &hit,
            0,
            &HashMap::new(),
            Language::SimplifiedChinese,
            &GenericTargetLabels::default(),
            &mut MainDpsDetailTextBudget::default(),
        );

        assert_eq!(snapshot.damage, 1_750.0);
        assert_eq!(snapshot.primary_damage, 1_500.0);
        assert_eq!(snapshot.overkill_damage, 500.0);
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
        assert_eq!(summary.candidate_hits, 1);
        assert_eq!(summary.confirmed_hits + summary.candidate_hits, 4);
        assert_eq!(summary.candidate_share_percent, 25.0);
    }

    #[test]
    fn live_detail_keeps_confirmed_and_candidate_hit_counts_disjoint() {
        let mut combat = CombatState::default();
        combat.push_hit(skill_hit(75.0, Some("GA_Confirmed"), None, "Skill"));
        let mut candidate = skill_hit(25.0, Some("GA_Candidate"), None, "Skill");
        candidate.timestamp = 2.0;
        candidate.direction = HitDirection::Unknown;
        combat.push_hit(candidate);

        let state = AppState::default();
        state.restore_live_state_for_test(
            combat,
            nte_dps_tool::engine::model::CaptureQualitySource::Live,
        );
        let snapshot = MainDpsDetailSnapshot::from_state(
            &state,
            MainDpsDetailKind::Team,
            0,
            MAIN_DPS_DETAIL_DEFAULT_LIMIT,
        )
        .expect("mixed-direction detail snapshot");

        assert_eq!(snapshot.direction.confirmed_hits, 1);
        assert_eq!(snapshot.direction.candidate_hits, 1);
        assert_eq!(
            snapshot.direction.confirmed_hits + snapshot.direction.candidate_hits,
            snapshot.metrics.output_count
        );
    }

    #[test]
    fn empty_state_serializes_the_complete_detail_contract() {
        let snapshot = MainDpsDetailSnapshot::from_state(
            &AppState::default(),
            MainDpsDetailKind::Team,
            0,
            200,
        )
        .expect("healthy live-capture detail snapshot");
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
        )
        .expect("healthy live-capture detail snapshot");

        assert_eq!(snapshot.total_hits, 50_000);
        assert_eq!(snapshot.total_damage, 50_000.0);
        assert_eq!(snapshot.rows.len(), 10);
        assert_eq!(snapshot.rows[0].timestamp, 49_990.0);
    }

    #[test]
    fn oversized_external_hit_text_is_bounded_and_reported() {
        let oversized = "界".repeat(MAIN_DPS_DETAIL_MAX_TEXT_BYTES);
        let mut hit = skill_hit(1.0, None, None, "Skill");
        hit.char_id = 424_242;
        hit.char_name = oversized.clone();
        hit.damage_component = Some(oversized.clone());
        hit.target_name = Some(oversized.clone());
        hit.target_monster_id = Some(oversized);
        let mut combat = CombatState::default();
        combat.push_hit(hit);
        let state = AppState::default();
        state.restore_live_state_for_test(
            combat,
            nte_dps_tool::engine::model::CaptureQualitySource::Live,
        );

        let snapshot = MainDpsDetailSnapshot::from_state(
            &state,
            MainDpsDetailKind::Team,
            0,
            MAIN_DPS_DETAIL_PAGE_LIMIT,
        )
        .expect("bounded detail snapshot");

        assert!(snapshot.text_truncated);
        let row = snapshot.rows.first().expect("bounded hit row");
        for value in [
            row.character_name.as_str(),
            row.skill_id.as_str(),
            row.skill.as_str(),
            row.target.as_str(),
        ] {
            assert!(value.len() <= MAIN_DPS_DETAIL_MAX_TEXT_BYTES);
        }
        assert!(
            row.target_monster_id
                .as_ref()
                .is_none_or(|value| value.len() <= MAIN_DPS_DETAIL_MAX_TEXT_BYTES)
        );
    }

    #[test]
    fn escape_heavy_maximum_snapshot_stays_below_stream_delivery_budget() {
        let escape_heavy = "\u{0001}".repeat(MAIN_DPS_DETAIL_MAX_TEXT_BYTES);
        let mut combat = CombatState::default();
        for index in 0..MAIN_DPS_DETAIL_PAGE_LIMIT {
            let skill = format!(
                "{index:03}{}",
                "\u{0001}".repeat(MAIN_DPS_DETAIL_MAX_TEXT_BYTES - 3)
            );
            let mut hit = skill_hit(1.0, None, None, "Skill");
            hit.timestamp = index as f64;
            hit.char_id = 424_242;
            hit.char_name = escape_heavy.clone();
            hit.damage_component = Some(skill);
            hit.attack_type = None;
            hit.damage_attribute = Some(escape_heavy.clone());
            hit.target_name = Some(escape_heavy.clone());
            hit.target_monster_id = Some(escape_heavy.clone());
            combat.push_hit(hit);
        }
        let state = AppState::default();
        state
            .set_main_dps_detail_request(
                MainDpsDetailKind::Character,
                MainDpsDetailRequest {
                    character_id: Some(424_242),
                    ..Default::default()
                },
            )
            .expect("set character detail request");
        state.restore_live_state_for_test(
            combat,
            nte_dps_tool::engine::model::CaptureQualitySource::Live,
        );

        let snapshot = MainDpsDetailSnapshot::from_state(
            &state,
            MainDpsDetailKind::Character,
            0,
            MAIN_DPS_DETAIL_PAGE_LIMIT,
        )
        .expect("maximum detail snapshot");
        assert_eq!(snapshot.rows.len(), MAIN_DPS_DETAIL_PAGE_LIMIT);
        assert_eq!(snapshot.skills.len(), MAIN_DPS_DETAIL_SKILL_LIMIT);
        assert!(
            snapshot.text_truncated,
            "the cumulative text budget must report omission"
        );

        let delivery = serialize_stream_events(vec![snapshot])
            .expect("bounded escape-heavy snapshot must serialize");
        assert!(
            delivery.len() < MAX_STREAM_DELIVERY_BYTES,
            "MAIN_DPS_DETAIL_MAX_STREAM_BYTES={} limit={MAX_STREAM_DELIVERY_BYTES}",
            delivery.len()
        );
    }

    #[test]
    fn skill_summary_output_is_server_bounded_with_explicit_truncation() {
        let mut combat = CombatState::default();
        for index in 0..=MAIN_DPS_DETAIL_SKILL_LIMIT {
            let mut hit = skill_hit(1.0, Some("GA_Fixture"), Some("Fixture"), "Skill");
            hit.ability_name = Some(format!("GA_Fixture_{index}"));
            hit.timestamp = index as f64;
            combat.push_hit(hit);
        }
        let state = AppState::default();
        state
            .set_main_dps_detail_request(
                MainDpsDetailKind::Character,
                MainDpsDetailRequest {
                    character_id: Some(1010),
                    ..Default::default()
                },
            )
            .expect("set character detail request");
        state.restore_live_state_for_test(
            combat,
            nte_dps_tool::engine::model::CaptureQualitySource::Live,
        );

        let snapshot = MainDpsDetailSnapshot::from_state(
            &state,
            MainDpsDetailKind::Character,
            0,
            MAIN_DPS_DETAIL_DEFAULT_LIMIT,
        )
        .expect("healthy live-capture detail snapshot");

        assert_eq!(snapshot.skill_total_count, MAIN_DPS_DETAIL_SKILL_LIMIT + 1);
        assert_eq!(snapshot.skills.len(), MAIN_DPS_DETAIL_SKILL_LIMIT);
        assert!(snapshot.skills_truncated);
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

        let summaries = skill_summaries(hits.iter().copied(), 130.0, Language::SimplifiedChinese);
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
