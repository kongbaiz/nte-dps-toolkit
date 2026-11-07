use crate::contract::timeline::{MAX_TIMELINE_CHARACTERS, MAX_TIMELINE_ROLES_PER_BUCKET};
use nte_dps_tool::{
    core::{
        live_capture::LiveCaptureStatus,
        packets::{PacketsProjection, project_packets_since, project_recent_packets},
        skills::SkillsProjection,
        timeline::{TimelineProjectionOptions, project_timeline},
    },
    storage::{
        config::TimelineDpsViewMode,
        history::{load_history_index, load_history_record_by_id_for_interactive_selection},
    },
};

use super::*;

impl AppState {
    pub(crate) fn live_capture_status(&self) -> LiveCaptureStatus {
        self.0.live_capture.status()
    }

    pub(crate) fn combat_clock_health(
        &self,
    ) -> Result<nte_dps_tool::engine::model::CombatClockRuntimeHealth, CoreError> {
        self.0.live_capture.combat_clock_health()
    }

    pub(super) fn live_dps_time_mode(&self) -> Result<DpsTimeMode, CoreError> {
        Ok(match self.0.live_capture.quality_source()? {
            CaptureQualitySource::Plugin => DpsTimeMode::TimeStopAdjusted,
            CaptureQualitySource::Live => DpsTimeMode::RealTime,
            _ => self.ui_config().dps_time_mode(),
        })
    }

    pub(crate) fn main_presented_dps_time_mode(&self) -> Result<DpsTimeMode, CoreError> {
        let mode = self.presentation_mode_snapshot();
        self.presentation_dps_time_mode(&mode)
    }

    fn presentation_dps_time_mode(
        &self,
        mode: &PresentationModeSnapshot,
    ) -> Result<DpsTimeMode, CoreError> {
        if let Some(selected) = &mode.selected_round {
            return Ok(selected.dps_time_mode);
        }
        if let Some(paused) = &mode.paused {
            return Ok(paused.dps_time_mode);
        }
        self.live_dps_time_mode()
    }

    pub(crate) fn main_presented_combat_clock_health(
        &self,
    ) -> Result<CombatClockRuntimeHealth, CoreError> {
        self.with_main_presented_state(|state| state.combat_clock_health)
    }

    pub(crate) fn replay_running(&self) -> Result<bool, CoreError> {
        self.0.live_capture.replay_running()
    }

    pub(super) fn presentation_mode_snapshot(&self) -> PresentationModeSnapshot {
        let outgoing_revision = self.0.live_capture.outgoing_hit_revision();
        let history_revision = self.history_revision();
        let (mut mode, recovered) = self.0.presentation.lock_mode();
        let stale_selection = mode.selected_round.as_ref().is_some_and(|selection| {
            selection.history_revision != history_revision
                || (mode.paused.is_none() && outgoing_revision != mode.selected_outgoing_revision)
        });
        if stale_selection {
            mode.selected_round = None;
        }
        let snapshot = PresentationModeSnapshot {
            paused: mode.paused.clone(),
            selected_round: mode.selected_round.clone(),
        };
        drop(mode);
        if recovered || stale_selection {
            self.0.presentation.publish_technical_and_main();
        }
        snapshot
    }

    pub(crate) fn main_processing_paused(&self) -> bool {
        self.presentation_mode_snapshot().processing_paused()
    }

    pub(crate) fn main_paused_event_counts(&self) -> Result<(u64, u64), CoreError> {
        let paused = self.presentation_mode_snapshot().paused;
        let Some(paused) = paused else {
            return Ok((0, 0));
        };
        self.0.live_capture.with_state(|live| {
            let semantic = live
                .hits_generation
                .wrapping_sub(paused.state.hits_generation)
                .wrapping_add(
                    live.empty_curtain_generation
                        .wrapping_sub(paused.state.empty_curtain_generation),
                )
                .wrapping_add(
                    live.empty_curtain_characters_generation
                        .wrapping_sub(paused.state.empty_curtain_characters_generation),
                );
            let debug = live
                .packets_generation
                .wrapping_sub(paused.state.packets_generation);
            (semantic, debug)
        })
    }

    pub(crate) fn set_main_processing_paused(
        &self,
        paused: bool,
    ) -> Result<bool, PresentationError> {
        let (mut mode, recovered) = self.0.presentation.lock_mode();
        if recovered {
            drop(mode);
            self.0.presentation.publish_technical_and_main();
            return Err(PresentationError::StateUnavailable);
        }
        if mode.paused.is_some() == paused {
            return Ok(false);
        }
        if !paused {
            mode.paused = None;
            drop(mode);
            self.0.presentation.publish_technical_and_main();
            return Ok(true);
        }
        drop(mode);

        // The capture snapshot can be large. Build it without holding any
        // Presentation lock, then re-check the requested transition.
        let configured_mode = self.ui_config().dps_time_mode();
        let frozen = self
            .0
            .live_capture
            .with_packet_state(|packet_revision, _, state| {
                // with_packet_state holds event_gate and state: acquire provenance
                // next in the capture lock order, freezing it with this snapshot.
                let dps_time_mode = match self.0.live_capture.quality_source()? {
                    CaptureQualitySource::Plugin => DpsTimeMode::TimeStopAdjusted,
                    CaptureQualitySource::Live => DpsTimeMode::RealTime,
                    _ => configured_mode,
                };
                Ok(PausedPresentation {
                    dps_time_mode,
                    state: Arc::new(state.clone()),
                    packet_revision,
                })
            })
            .map_err(PresentationError::Capture)?
            .map_err(PresentationError::Capture)?;
        let (mut mode, recovered) = self.0.presentation.lock_mode();
        if recovered {
            drop(mode);
            self.0.presentation.publish_technical_and_main();
            return Err(PresentationError::StateUnavailable);
        }
        if mode.paused.is_some() {
            return Ok(false);
        }
        mode.paused = Some(frozen);
        drop(mode);
        self.0.presentation.publish_technical_and_main();
        Ok(true)
    }

    pub(crate) fn main_selected_round_id(&self) -> Option<String> {
        self.presentation_mode_snapshot().selected_round_id()
    }

    pub(crate) fn set_main_selected_round_id(
        &self,
        record_id: Option<String>,
    ) -> Result<bool, PresentationError> {
        let operation = self.reserve_main_round_selection();
        self.set_main_selected_round_id_for_operation(record_id, operation)
    }

    pub(crate) fn reserve_main_round_selection(&self) -> u64 {
        self.0
            .presentation
            .selection_generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
    }

    pub(crate) fn set_main_selected_round_id_for_operation(
        &self,
        record_id: Option<String>,
        operation: u64,
    ) -> Result<bool, PresentationError> {
        self.set_main_selected_round_id_with(record_id, operation, |record_id| {
            let mut record = load_history_record_by_id_for_interactive_selection(record_id)
                .map_err(|error| {
                    match error {
                    nte_dps_tool::storage::history::HistoryRecordLoadError::DetailsTooLarge {
                        ..
                    }
                    | nte_dps_tool::storage::history::HistoryRecordLoadError::DetailHitsTooLarge {
                        ..
                    } => PresentationError::RoundTooLarge,
                    _ => PresentationError::RoundUnavailable,
                }
                })?
                .filter(|record| record.details.is_some())
                .ok_or(PresentationError::RoundUnavailable)?;
            record
                .details
                .take()
                .map(|details| {
                    (
                        details.into_combat_state(),
                        if record.summary.dps_time_mode.subtracts_time_stop() {
                            DpsTimeMode::TimeStopAdjusted
                        } else {
                            DpsTimeMode::RealTime
                        },
                    )
                })
                .ok_or(PresentationError::RoundUnavailable)
        })
    }

    pub(super) fn set_main_selected_round_id_with(
        &self,
        record_id: Option<String>,
        operation: u64,
        load: impl FnOnce(&str) -> Result<(CombatState, DpsTimeMode), PresentationError>,
    ) -> Result<bool, PresentationError> {
        let history_revision = self.history_revision();
        if self
            .0
            .presentation
            .selection_generation
            .load(Ordering::Acquire)
            != operation
        {
            return Ok(false);
        }
        let (mode, recovered) = self.0.presentation.lock_mode();
        if recovered {
            drop(mode);
            self.0.presentation.publish_technical_and_main();
            return Err(PresentationError::StateUnavailable);
        }
        if mode.selected_round.as_ref().is_some_and(|value| {
            Some(value.record_id.as_str()) == record_id.as_deref()
                && value.history_revision == history_revision
        }) || (record_id.is_none() && mode.selected_round.is_none())
        {
            return Ok(false);
        }
        drop(mode);

        // History loading and reconstruction are blocking/O(N) and remain
        // outside every Presentation lock. Superseded failures are ignored:
        // their newer intent owns both the visible state and command snapshot.
        let selection = match record_id {
            Some(record_id) => {
                let (state, dps_time_mode) = match load(&record_id) {
                    Ok(state) => state,
                    Err(_error)
                        if self
                            .0
                            .presentation
                            .selection_generation
                            .load(Ordering::Acquire)
                            != operation =>
                    {
                        return Ok(false);
                    }
                    Err(error) => return Err(error),
                };
                Some(SelectedRoundPresentation {
                    dps_time_mode,
                    record_id,
                    history_revision,
                    state: Arc::new(state),
                })
            }
            None => None,
        };
        let (mut mode, recovered) = self.0.presentation.lock_mode();
        if recovered {
            drop(mode);
            self.0.presentation.publish_technical_and_main();
            return Err(PresentationError::StateUnavailable);
        }
        if self.history_revision() != history_revision
            || self
                .0
                .presentation
                .selection_generation
                .load(Ordering::Acquire)
                != operation
        {
            return Ok(false);
        }
        if mode
            .selected_round
            .as_ref()
            .map(|value| (value.record_id.as_str(), value.history_revision))
            == selection
                .as_ref()
                .map(|value| (value.record_id.as_str(), value.history_revision))
        {
            return Ok(false);
        }
        mode.selected_outgoing_revision = self.0.live_capture.outgoing_hit_revision();
        mode.selected_round = selection;
        drop(mode);
        self.0.presentation.publish_technical_and_main();
        Ok(true)
    }

    pub(crate) fn main_round_index(&self) -> Arc<Vec<HistoryRoundIndex>> {
        let index = self.load_main_round_index();
        let stale_selection = self
            .main_selected_round_id()
            .is_some_and(|id| !index.iter().any(|record| record.id == id));
        if stale_selection {
            let _ = self.set_main_selected_round_id(None);
        }
        index
    }

    fn load_main_round_index(&self) -> Arc<Vec<HistoryRoundIndex>> {
        let revision = self.history_revision();
        let cache = lock_or_reset(
            &self.0.history.round_cache,
            "History round index cache was discarded after an interrupted update",
        )
        .0;
        if cache.revision == Some(revision) {
            return Arc::clone(&cache.index);
        }
        drop(cache);

        // Directory scan and JSON parsing stay outside the cache mutex. A
        // concurrent history mutation makes this projection non-cacheable but
        // does not justify holding a UI lock across disk I/O.
        let loaded = Arc::new(
            load_history_index()
                .records
                .iter()
                .map(HistoryRoundIndex::from_storage)
                .collect(),
        );
        let mut cache = lock_or_reset(
            &self.0.history.round_cache,
            "History round index cache was discarded after an interrupted update",
        )
        .0;
        if self.history_revision() == revision {
            cache.index = Arc::clone(&loaded);
            cache.revision = Some(revision);
        }
        loaded
    }

    pub(crate) fn main_dps_readout(&self) -> Result<MainDpsReadout, CoreError> {
        const MAIN_READOUT_CACHE_CAPACITY: usize = 6;
        let mode = self.presentation_mode_snapshot();
        let follow_live_half = mode.selected_round.is_none() && !mode.processing_paused();
        let abyss_before = self.abyss_presentation_snapshot();
        let abyss_after = self.with_presentation_mode_state(&mode, |state| {
            if !follow_live_half {
                return abyss_before;
            }
            let (selected, observed) = next_live_abyss_selection(
                abyss_before.selected,
                abyss_before.observed,
                state.abyss.active_half,
            );
            AbyssPresentationState { selected, observed }
        })?;
        if follow_live_half {
            self.commit_follow_live_abyss(abyss_before, abyss_after);
        }

        let revision = self.main_dps_stream_revision()?;
        let key = MainReadoutCacheKey { revision };
        let (cache, recovered) = self.0.presentation.lock_main_readout_cache();
        if !recovered && let Some(cached) = cache.iter().find(|cached| cached.key == key) {
            return Ok(cached.readout.as_ref().clone());
        }
        drop(cache);

        let dps_time_mode = self.presentation_dps_time_mode(&mode)?;
        let readout = self.with_presentation_mode_state(&mode, |state| {
            self.project_main_readout(state, abyss_after.selected, dps_time_mode)
        })?;

        // A concurrent hit/settings/presentation mutation makes this result a
        // valid one-shot snapshot, but it must not be published under an older
        // key. The next consumer projects the newer exact revision instead.
        if self.main_dps_stream_revision()? != revision {
            return Ok(readout);
        }
        let readout = Arc::new(readout);
        let (mut cache, _) = self.0.presentation.lock_main_readout_cache();
        if let Some(cached) = cache.iter().find(|cached| cached.key == key) {
            return Ok(cached.readout.as_ref().clone());
        }
        cache.push(MainReadoutCache {
            key,
            readout: Arc::clone(&readout),
        });
        if cache.len() > MAIN_READOUT_CACHE_CAPACITY {
            let remove_count = cache.len() - MAIN_READOUT_CACHE_CAPACITY;
            cache.drain(..remove_count);
        }
        drop(cache);
        let readout = readout.as_ref().clone();
        Ok(readout)
    }

    pub(crate) fn main_dps_detail_request(&self, kind: MainDpsDetailKind) -> MainDpsDetailRequest {
        let (details, recovered) = self.0.presentation.lock_details();
        let request = match kind {
            MainDpsDetailKind::Character => details.character.clone(),
            MainDpsDetailKind::Team => details.team.clone(),
        };
        drop(details);
        if recovered {
            self.0.presentation.publish_main();
        }
        request
    }

    pub(crate) fn main_dps_detail_cache_get(
        &self,
        revision: MainDpsStreamRevision,
        kind: MainDpsDetailKind,
        request: &MainDpsDetailRequest,
        offset: usize,
        limit: usize,
    ) -> Option<Arc<MainDpsDetailSnapshot>> {
        let key = MainDpsDetailCacheKey {
            revision,
            kind,
            request: request.clone(),
            offset,
            limit,
        };
        let (cache, recovered) = self.0.presentation.lock_cache();
        if recovered {
            return None;
        }
        cache
            .iter()
            .find(|cache| cache.key == key)
            .map(|cache| Arc::clone(&cache.snapshot))
    }

    pub(crate) fn main_dps_detail_cache_store(
        &self,
        revision: MainDpsStreamRevision,
        kind: MainDpsDetailKind,
        request: &MainDpsDetailRequest,
        offset: usize,
        limit: usize,
        snapshot: Arc<MainDpsDetailSnapshot>,
    ) {
        let key = MainDpsDetailCacheKey {
            revision,
            kind,
            request: request.clone(),
            offset,
            limit,
        };
        let (mut cache, _) = self.0.presentation.lock_cache();
        cache.retain(|entry| entry.key != key);
        cache.push(MainDpsDetailCache { key, snapshot });
        if cache.len() > MAIN_DPS_DETAIL_CACHE_CAPACITY {
            let remove_count = cache.len() - MAIN_DPS_DETAIL_CACHE_CAPACITY;
            cache.drain(..remove_count);
        }
    }

    pub(crate) fn set_main_dps_detail_request(
        &self,
        kind: MainDpsDetailKind,
        request: MainDpsDetailRequest,
    ) -> Result<bool, PresentationError> {
        let (mut details, recovered) = self.0.presentation.lock_details();
        if recovered {
            drop(details);
            self.0.presentation.publish_main();
            return Err(PresentationError::StateUnavailable);
        }
        let current = match kind {
            MainDpsDetailKind::Character => &mut details.character,
            MainDpsDetailKind::Team => &mut details.team,
        };
        if *current == request {
            return Ok(false);
        }
        *current = request;
        drop(details);
        self.0.presentation.publish_main();
        Ok(true)
    }

    pub(super) fn abyss_presentation_snapshot(&self) -> AbyssPresentationState {
        let (abyss, recovered) = self.0.presentation.lock_abyss();
        let snapshot = *abyss;
        drop(abyss);
        if recovered {
            self.0.presentation.publish_technical_and_main();
        }
        snapshot
    }

    pub(crate) fn with_main_dps_detail_state<T>(
        &self,
        project: impl FnOnce(&CombatState, Option<AbyssHalf>) -> T,
    ) -> Result<T, CoreError> {
        let selected_abyss_half = self.abyss_presentation_snapshot().selected;
        self.with_main_presented_state(|state| {
            let selected_half = state.abyss.is_active().then(|| {
                selected_abyss_half
                    .or(state.abyss.active_half)
                    .unwrap_or(AbyssHalf::First)
            });
            project(state, selected_half)
        })
    }

    pub(crate) fn set_main_selected_abyss_half(
        &self,
        half: Option<AbyssHalf>,
    ) -> Result<bool, PresentationError> {
        let (mut abyss, recovered) = self.0.presentation.lock_abyss();
        if recovered {
            drop(abyss);
            self.0.presentation.publish_technical_and_main();
            return Err(PresentationError::StateUnavailable);
        }
        if abyss.selected == half {
            return Ok(false);
        }
        abyss.selected = half;
        drop(abyss);
        self.0.presentation.publish_technical_and_main();
        Ok(true)
    }

    pub(crate) fn update_main_appearance(
        &self,
        dark_mode: bool,
        opacity: f32,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::SETTINGS_AND_MAIN, |config| {
            config.dark_mode = dark_mode;
            config.opacity = opacity;
        })
    }

    pub(crate) fn main_window_geometry(&self) -> (Option<[f32; 2]>, Option<[f32; 2]>) {
        let config = self.ui_config();
        (config.main_window_size, config.main_window_position)
    }

    pub(crate) fn set_main_window_geometry(
        &self,
        size: Option<[f32; 2]>,
        position: Option<[f32; 2]>,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::NONE, |config| {
            config.main_window_size = size;
            config.main_window_position = position;
        })
    }

    pub(crate) fn main_dps_stream_revision(&self) -> Result<MainDpsStreamRevision, CoreError> {
        let mode = self.presentation_mode_snapshot();
        let _ = self.abyss_presentation_snapshot();
        let _ = self.main_dps_detail_request(MainDpsDetailKind::Character);
        let selected_history = mode.selected_round.is_some();
        let paused = mode.processing_paused();
        Ok(MainDpsStreamRevision {
            capture: if selected_history {
                0
            } else {
                self.0.live_capture.revision()
            },
            packet: if paused && !selected_history {
                self.0
                    .live_capture
                    .with_packet_state(|revision, _, _| revision.generation)?
            } else {
                0
            },
            presentation: self.0.presentation.revision.load(Ordering::Acquire),
            history: self.history_revision(),
            main: self.0.presentation.main_revision.load(Ordering::Acquire),
        })
    }

    fn project_main_readout(
        &self,
        state: &CombatState,
        selected_abyss_half: Option<AbyssHalf>,
        dps_time_mode: DpsTimeMode,
    ) -> MainDpsReadout {
        let config = self.ui_config();
        let subtract_time_stop = subtract_time_stop_for_state(dps_time_mode, state);
        let projection_half = state.abyss.is_active().then(|| {
            selected_abyss_half
                .or(state.abyss.active_half)
                .unwrap_or(AbyssHalf::First)
        });
        let hud = project_hud(
            state,
            &HudConfig::detailed(),
            &HashSet::new(),
            HudProjectionOptions {
                dps_time_basis: DpsTimeBasis::from_subtract_time_stop(subtract_time_stop),
                separate_reaction_damage: config.separate_reaction_damage,
                include_max_hp_reduction_in_total_damage: config
                    .include_max_hp_reduction_in_total_damage,
                selected_abyss_half,
                preview_when_empty: false,
                timeline_bucket_seconds: f64::from(sanitize_timeline_bucket_seconds(
                    config.timeline_bucket_seconds,
                )),
            },
        );
        let (damage_attribution, character_durations) = projection_half.map_or_else(
            || {
                let durations = hud
                    .characters
                    .iter()
                    .filter_map(|row| {
                        state.stats.get(&row.character_id).map(|stats| {
                            (
                                row.character_id,
                                state.character_duration_with_time_stop(stats, subtract_time_stop),
                            )
                        })
                    })
                    .collect();
                (
                    state
                        .damage_attribution_summary()
                        .with_max_hp_reduction_in_total(
                            config.include_max_hp_reduction_in_total_damage,
                        ),
                    durations,
                )
            },
            |half| {
                let party = state.abyss.half(half);
                let durations = hud
                    .characters
                    .iter()
                    .filter_map(|row| {
                        party.stats.get(&row.character_id).map(|stats| {
                            (
                                row.character_id,
                                party.character_duration_with_time_stop(stats, subtract_time_stop),
                            )
                        })
                    })
                    .collect();
                (
                    party
                        .damage_attribution_summary()
                        .with_max_hp_reduction_in_total(
                            config.include_max_hp_reduction_in_total_damage,
                        ),
                    durations,
                )
            },
        );
        MainDpsReadout {
            hud,
            has_hits: !state.hits.is_empty(),
            game_paused: state.is_game_paused(),
            damage_attribution,
            separate_reaction_damage: config.separate_reaction_damage,
            character_durations,
        }
    }

    fn commit_follow_live_abyss(
        &self,
        expected: AbyssPresentationState,
        candidate: AbyssPresentationState,
    ) {
        if expected == candidate {
            return;
        }
        let (mut abyss, recovered) = self.0.presentation.lock_abyss();
        if recovered {
            drop(abyss);
            self.0.presentation.publish_technical_and_main();
            return;
        }
        // An explicit user selection wins over a stale projection. The next
        // capture revision will compute another follow-live candidate.
        if *abyss == expected {
            *abyss = candidate;
        }
    }

    pub(super) fn bump_main_dps_revision(&self) -> u64 {
        self.0
            .presentation
            .main_revision
            .fetch_add(1, Ordering::AcqRel)
            + 1
    }

    pub(crate) fn timeline_projection(
        &self,
        scope: TimelineScope,
    ) -> Result<Arc<TimelineProjection>, CoreError> {
        const TIMELINE_CACHE_CAPACITY: usize = 6;
        let config = self.ui_config();
        let dps_time_mode = self.main_presented_dps_time_mode()?;
        let revision = self.main_dps_stream_revision()?;
        let subtract_time_stop = matches!(dps_time_mode, DpsTimeMode::TimeStopAdjusted)
            && matches!(
                self.main_presented_combat_clock_health()?,
                CombatClockRuntimeHealth::Available | CombatClockRuntimeHealth::Recorded
            );
        let key = TimelineProjectionCacheKey {
            revision,
            scope,
            bucket_seconds_bits: config.timeline_bucket_seconds.to_bits(),
            subtract_time_stop,
            language: config.language.code(),
        };
        let (cache, recovered) = self.0.presentation.lock_timeline_cache();
        if !recovered && let Some(cached) = cache.iter().find(|cached| cached.key == key) {
            return Ok(Arc::clone(&cached.projection));
        }
        drop(cache);

        let resources = self.0.live_capture.resources();
        let projection = Arc::new(self.with_main_presented_state(|state| {
            project_timeline(
                state,
                &resources.characters,
                TimelineProjectionOptions {
                    scope,
                    bucket_seconds: config.timeline_bucket_seconds,
                    max_buckets: MAX_TIMELINE_BUCKETS,
                    max_roles_per_bucket: MAX_TIMELINE_ROLES_PER_BUCKET,
                    max_characters: MAX_TIMELINE_CHARACTERS,
                    subtract_time_stop,
                    language: config.language,
                },
            )
        })?);

        let (mut cache, _) = self.0.presentation.lock_timeline_cache();
        if let Some(cached) = cache.iter().find(|cached| cached.key == key) {
            return Ok(Arc::clone(&cached.projection));
        }
        cache.push(TimelineProjectionCache {
            key,
            projection: Arc::clone(&projection),
        });
        if cache.len() > TIMELINE_CACHE_CAPACITY {
            let remove_count = cache.len() - TIMELINE_CACHE_CAPACITY;
            cache.drain(..remove_count);
        }
        Ok(projection)
    }

    pub(crate) fn packet_stream_revision(&self) -> Result<PacketStreamRevision, CoreError> {
        if let Some(paused) = self.presentation_mode_snapshot().paused {
            return Ok(paused.packet_revision);
        }
        self.0
            .live_capture
            .with_packet_state(|revision, _, _| revision)
    }

    pub(crate) fn packets_projection(
        &self,
        after: Option<PacketStreamRevision>,
    ) -> Result<(PacketStreamRevision, bool, PacketsProjection), CoreError> {
        if let Some(paused) = self.presentation_mode_snapshot().paused {
            let revision = paused.packet_revision;
            let incremental = after
                .filter(|cursor| cursor.session_generation == revision.session_generation)
                .and_then(|cursor| {
                    project_packets_since(&paused.state, revision, 0, cursor.packet_generation)
                });
            return Ok(match incremental {
                Some(projection) => (revision, false, projection),
                None => (
                    revision,
                    true,
                    project_recent_packets(&paused.state, revision, 0),
                ),
            });
        }
        self.0
            .live_capture
            .with_packet_state(|revision, queued_event_count, state| {
                let incremental = after
                    .filter(|cursor| cursor.session_generation == revision.session_generation)
                    .and_then(|cursor| {
                        project_packets_since(
                            state,
                            revision,
                            queued_event_count,
                            cursor.packet_generation,
                        )
                    });
                match incremental {
                    Some(projection) => (revision, false, projection),
                    None => (
                        revision,
                        true,
                        project_recent_packets(state, revision, queued_event_count),
                    ),
                }
            })
    }

    pub(crate) fn skills_projection(
        &self,
        scope: TimelineScope,
    ) -> Result<SkillsProjection, CoreError> {
        let config = self.ui_config();
        let resources = self.0.live_capture.resources();
        self.with_main_presented_state(|state| {
            project_skills(
                state,
                &resources.characters,
                SkillsProjectionOptions {
                    scope,
                    language: config.language,
                },
            )
        })
    }

    pub(crate) fn timeline_preferences(&self) -> (f32, TimelineDpsViewMode) {
        let config = self.ui_config();
        (
            sanitize_timeline_bucket_seconds(config.timeline_bucket_seconds),
            config.timeline_dps_view_mode,
        )
    }

    pub(crate) fn update_timeline_preferences(
        &self,
        bucket_seconds: f32,
        view_mode: TimelineDpsViewMode,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(
            SettingsMutationEffects {
                technical: true,
                ..SettingsMutationEffects::NONE
            },
            |config| {
                config.timeline_bucket_seconds = bucket_seconds;
                config.timeline_dps_view_mode = view_mode;
            },
        )
    }

    pub(super) fn with_main_presented_state<T>(
        &self,
        project: impl FnOnce(&CombatState) -> T,
    ) -> Result<T, CoreError> {
        let mode = self.presentation_mode_snapshot();
        self.with_presentation_mode_state(&mode, project)
    }

    fn with_presentation_mode_state<T>(
        &self,
        mode: &PresentationModeSnapshot,
        project: impl FnOnce(&CombatState) -> T,
    ) -> Result<T, CoreError> {
        if let Some(state) = mode.presented_state() {
            return Ok(project(state.as_ref()));
        }
        self.0.live_capture.with_state(project)
    }

    pub(super) fn return_main_presentation_to_live(&self) {
        let outgoing_revision = self.0.live_capture.outgoing_hit_revision();
        let (mut mode, recovered_mode) = self.0.presentation.lock_mode();
        let mode_changed = mode.paused.is_some() || mode.selected_round.is_some();
        mode.paused = None;
        mode.selected_round = None;
        mode.selected_outgoing_revision = outgoing_revision;
        drop(mode);

        let (mut abyss, recovered_abyss) = self.0.presentation.lock_abyss();
        let abyss_changed = *abyss != AbyssPresentationState::default();
        *abyss = AbyssPresentationState::default();
        drop(abyss);

        if recovered_mode || recovered_abyss || mode_changed || abyss_changed {
            self.0.presentation.publish_technical_and_main();
        }
    }
}
