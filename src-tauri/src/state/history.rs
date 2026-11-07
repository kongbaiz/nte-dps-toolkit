use nte_dps_tool::{
    core::history::{PendingHistoryArchive, auto_round_due, prepare_history_archive_owned},
    storage::history::save_borrowed_archive_outcome,
};

use super::*;

impl AppState {
    pub(crate) fn prepare_current_history_archive(
        &self,
    ) -> Result<Option<PreparedHistoryArchive>, CoreError> {
        let config = self.ui_config();
        let (state, source) = self.0.live_capture.history_state_and_source_snapshot()?;
        let dps_time_mode = DpsTimeBasis::from_subtract_time_stop(subtract_time_stop_for_state(
            match source {
                CaptureQualitySource::Plugin => DpsTimeMode::TimeStopAdjusted,
                CaptureQualitySource::Live => DpsTimeMode::RealTime,
                _ => config.dps_time_mode(),
            },
            &state,
        ));
        Ok(prepare_history_archive_owned(
            state,
            source,
            dps_time_mode,
            config.separate_reaction_damage,
        ))
    }

    fn persist_history_archive(
        &self,
        archive: &PreparedHistoryArchive,
    ) -> Result<HistoryPersistenceOutcome, HistoryRuntimeError> {
        self.with_history_transaction(|| self.persist_history_archive_locked(archive))?
    }

    fn persist_history_archive_locked(
        &self,
        archive: &PreparedHistoryArchive,
    ) -> Result<HistoryPersistenceOutcome, HistoryRuntimeError> {
        self.persist_history_parts_locked(&archive.summary, archive.details.as_ref())
    }

    fn persist_pending_history_archive(
        &self,
        archive: &PendingHistoryArchive,
    ) -> Result<HistoryPersistenceOutcome, HistoryRuntimeError> {
        self.with_history_transaction(|| {
            self.persist_history_parts_locked(&archive.summary, Some(&archive.details))
        })?
    }

    fn persist_history_parts_locked(
        &self,
        summary: &nte_dps_tool::engine::model::CombatSessionSummary,
        details: Option<&HistoryCombatDetails>,
    ) -> Result<HistoryPersistenceOutcome, HistoryRuntimeError> {
        let outcome = save_borrowed_archive_outcome(summary, details).map_err(|error| {
            log::warn!("History archive storage operation did not finish: {error}");
            if error.is_retryable() {
                HistoryRuntimeError::Persistence
            } else {
                HistoryRuntimeError::PermanentPersistence
            }
        })?;
        Ok(self.finish_borrowed_history_save(outcome))
    }

    pub(super) fn finish_borrowed_history_save(
        &self,
        outcome: BorrowedHistorySaveOutcome,
    ) -> HistoryPersistenceOutcome {
        let persistence_outcome = match outcome {
            BorrowedHistorySaveOutcome::Committed => HistoryPersistenceOutcome::Committed,
            BorrowedHistorySaveOutcome::CommittedWithMaintenanceWarning(warning) => {
                log::warn!("{warning}");
                HistoryPersistenceOutcome::CommittedWithMaintenanceWarning
            }
        };
        self.bump_history_revision();
        persistence_outcome
    }

    pub(crate) fn archive_current_history_round(&self) -> Result<bool, HistoryRuntimeError> {
        if self
            .replay_running()
            .map_err(|_| HistoryRuntimeError::Unavailable)?
        {
            return Err(HistoryRuntimeError::ReplayActive);
        }
        self.archive_current_history_round_with(false, |state, archive| {
            state.persist_history_archive_locked(archive)
        })
    }

    pub(super) fn archive_current_history_round_with(
        &self,
        only_if_idle: bool,
        persist: impl FnOnce(
            &Self,
            &PreparedHistoryArchive,
        ) -> Result<HistoryPersistenceOutcome, HistoryRuntimeError>,
    ) -> Result<bool, HistoryRuntimeError> {
        let _archive_transaction = self
            .0
            .history
            .archive_transaction
            .lock()
            .map_err(|_| HistoryRuntimeError::Unavailable)?;
        // Lock order is archive_transaction -> transaction ->
        // pending_archives. Commands acquire only transaction (then undo), so
        // no inverse order exists. Both serialization guards are acquired
        // before capture state is inspected or detached.
        let _history_transaction = self.lock_history_transaction()?;
        if self
            .0
            .history
            .pending_archives
            .lock()
            .map_err(|_| HistoryRuntimeError::Unavailable)?
            .len()
            >= MAX_PENDING_HISTORY_ARCHIVES
        {
            return Err(HistoryRuntimeError::RetryQueueFull);
        }
        let config = self.ui_config();
        let policy = history_archive_policy(&config);
        let cut = if only_if_idle {
            if !config.auto_round_after_idle {
                return Ok(false);
            }
            self.0
                .live_capture
                .cut_idle_round(policy, config.auto_round_idle_seconds)
        } else {
            self.0.live_capture.cut_round_with_history_policy(policy)
        };
        let Some(cut) = cut.map_err(|_| HistoryRuntimeError::Unavailable)? else {
            return Ok(false);
        };
        let Some(archive) = prepare_history_archive_owned(
            cut.state,
            cut.source,
            cut.dps_time_mode,
            cut.separate_reaction_damage,
        ) else {
            log::warn!("cut History round had no archivable summary");
            return Ok(true);
        };
        if let Err(error) = persist(self, &archive) {
            log::warn!("History persistence failed after the live round was cut: {error}");
            if matches!(
                error,
                HistoryRuntimeError::Unavailable | HistoryRuntimeError::Persistence
            ) {
                self.0
                    .history
                    .pending_archives
                    .lock()
                    .map_err(|_| HistoryRuntimeError::Unavailable)?
                    .push_back(archive);
            } else {
                return Err(error);
            }
        }
        Ok(true)
    }

    pub(crate) fn maintain_history_rounds(&self) -> Result<(), HistoryRuntimeError> {
        self.retry_pending_history_archives()?;
        let mut pending = self
            .0
            .live_capture
            .take_pending_abyss_archives()
            .map_err(|_| HistoryRuntimeError::Unavailable)?
            .into_iter();
        let mut retry = Vec::new();
        while let Some(current) = pending.next() {
            match self.persist_pending_history_archive(&current) {
                Ok(_) => {}
                Err(HistoryRuntimeError::Unavailable) => {
                    retry.push(current);
                    retry.extend(pending);
                    self.0
                        .live_capture
                        .restore_pending_abyss_archives(retry)
                        .map_err(|_| HistoryRuntimeError::Unavailable)?;
                    return Err(HistoryRuntimeError::Unavailable);
                }
                Err(HistoryRuntimeError::Persistence) => {
                    retry.push(current);
                }
                Err(error) => {
                    log::warn!("automatic Abyss History archive failed: {error}");
                }
            }
        }
        if !retry.is_empty() {
            self.0
                .live_capture
                .restore_pending_abyss_archives(retry)
                .map_err(|_| HistoryRuntimeError::Unavailable)?;
            return Ok(());
        }

        let config = self.ui_config();
        if !config.auto_round_after_idle {
            return Ok(());
        }
        let status = self.0.live_capture.status();
        let replay_running = self
            .replay_running()
            .map_err(|_| HistoryRuntimeError::Unavailable)?;
        let idle_elapsed = self
            .0
            .live_capture
            .idle_elapsed()
            .map_err(|_| HistoryRuntimeError::Unavailable)?;
        let due = self
            .0
            .live_capture
            .with_state(|state| {
                auto_round_due(
                    status.phase == LiveCapturePhase::Running && !replay_running,
                    false,
                    state.abyss.is_active(),
                    state.is_game_paused(),
                    !state.hits.is_empty(),
                    idle_elapsed,
                    config.auto_round_idle_seconds,
                )
            })
            .map_err(|_| HistoryRuntimeError::Unavailable)?;
        if due {
            // This probe is only a hint; recheck after waiting for the History locks.
            match self.archive_current_history_round_with(true, |state, archive| {
                state.persist_history_archive_locked(archive)
            }) {
                Ok(_) => {}
                Err(HistoryRuntimeError::Unavailable) => {
                    return Err(HistoryRuntimeError::Unavailable);
                }
                Err(error) => {
                    log::warn!("automatic idle History archive failed: {error}");
                }
            }
        }
        Ok(())
    }

    fn retry_pending_history_archives(&self) -> Result<(), HistoryRuntimeError> {
        self.retry_pending_history_archives_with(|state, archive| {
            state.persist_history_archive(archive)
        })
    }

    pub(super) fn retry_pending_history_archives_with(
        &self,
        mut persist: impl FnMut(
            &Self,
            &PreparedHistoryArchive,
        ) -> Result<HistoryPersistenceOutcome, HistoryRuntimeError>,
    ) -> Result<(), HistoryRuntimeError> {
        let _archive_transaction = self
            .0
            .history
            .archive_transaction
            .lock()
            .map_err(|_| HistoryRuntimeError::Unavailable)?;
        // Check the serializer even when the retry queue is empty so a
        // poisoned maintenance runtime terminates. Release it immediately;
        // each archive persistence acquires its own transaction guard.
        drop(self.lock_history_transaction()?);
        let pending = {
            let mut pending = self
                .0
                .history
                .pending_archives
                .lock()
                .map_err(|_| HistoryRuntimeError::Unavailable)?;
            std::mem::take(&mut *pending)
        };
        if pending.is_empty() {
            return Ok(());
        }

        let mut pending = pending.into_iter();
        let mut retry = VecDeque::new();
        while let Some(archive) = pending.next() {
            match persist(self, &archive) {
                Ok(_) => {}
                Err(HistoryRuntimeError::Unavailable) => {
                    retry.push_back(archive);
                    retry.extend(pending);
                    self.restore_history_retry_queue(retry)?;
                    return Err(HistoryRuntimeError::Unavailable);
                }
                Err(HistoryRuntimeError::Persistence) => {
                    log::warn!("retrying a pending History archive failed");
                    retry.push_back(archive);
                }
                Err(error) => {
                    log::warn!("retrying a pending History archive failed: {error}");
                }
            }
        }
        if retry.is_empty() {
            return Ok(());
        }

        self.restore_history_retry_queue(retry)
    }

    fn restore_history_retry_queue(
        &self,
        mut retry: VecDeque<PreparedHistoryArchive>,
    ) -> Result<(), HistoryRuntimeError> {
        let mut pending = self
            .0
            .history
            .pending_archives
            .lock()
            .map_err(|_| HistoryRuntimeError::Unavailable)?;
        retry.append(&mut *pending);
        *pending = retry;
        Ok(())
    }

    pub(crate) fn with_history_transaction<T>(
        &self,
        action: impl FnOnce() -> T,
    ) -> Result<T, HistoryRuntimeError> {
        let _guard = self.lock_history_transaction()?;
        Ok(action())
    }

    fn lock_history_transaction(&self) -> Result<MutexGuard<'_, ()>, HistoryRuntimeError> {
        self.0
            .history
            .transaction
            .lock()
            .map_err(|_| HistoryRuntimeError::Unavailable)
    }

    pub(crate) fn history_revision(&self) -> u64 {
        self.0.history.revision.load(Ordering::Acquire)
    }

    pub(crate) fn bump_history_revision(&self) -> u64 {
        self.0.history.revision.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub(crate) fn ensure_history_undo_available(&self) -> Result<(), HistoryRuntimeError> {
        drop(
            self.0
                .history
                .undo
                .lock()
                .map_err(|_| HistoryRuntimeError::Unavailable)?,
        );
        Ok(())
    }

    pub(crate) fn remember_deleted_history_tombstone(
        &self,
        tombstone: HistoryDeleteTombstone,
    ) -> Result<String, HistoryRuntimeError> {
        let token = tombstone.token().to_owned();
        let replaced = {
            let mut undo = self
                .0
                .history
                .undo
                .lock()
                .map_err(|_| HistoryRuntimeError::Unavailable)?;
            undo.replace(HistoryUndoEntry {
                token: token.clone(),
                tombstone,
                expires_at: Instant::now() + HISTORY_UNDO_WINDOW,
            })
        };
        // Disk cleanup is deliberately outside the undo mutex. History command
        // callers hold the broader transaction guard, so replacement remains
        // serialized while slow filesystem work cannot block token reads.
        if let Some(replaced) = replaced
            && let Err(error) = discard_tombstoned_record(&replaced.tombstone)
        {
            log::warn!("replaced History undo tombstone cleanup did not finish: {error}");
        }
        Ok(token)
    }

    pub(crate) fn peek_deleted_history_tombstone(
        &self,
        token: &str,
    ) -> Result<Option<HistoryDeleteTombstone>, HistoryRuntimeError> {
        let (active, expired) = {
            let mut undo = self
                .0
                .history
                .undo
                .lock()
                .map_err(|_| HistoryRuntimeError::Unavailable)?;
            if undo
                .as_ref()
                .is_some_and(|entry| Instant::now() > entry.expires_at)
            {
                (None, undo.take())
            } else {
                (
                    undo.as_ref()
                        .filter(|entry| entry.token == token)
                        .map(|entry| entry.tombstone.clone()),
                    None,
                )
            }
        };
        // Expiry cleanup is also outside the mutex. A failed cleanup becomes a
        // bounded orphan for startup/maintenance cleanup; it is never restored
        // as a valid undo entry after its deadline.
        if let Some(expired) = expired
            && let Err(error) = discard_tombstoned_record(&expired.tombstone)
        {
            log::warn!("expired History undo tombstone cleanup did not finish: {error}");
        }
        Ok(active)
    }

    pub(crate) fn consume_deleted_history(&self, token: &str) -> Result<bool, HistoryRuntimeError> {
        let mut undo = self
            .0
            .history
            .undo
            .lock()
            .map_err(|_| HistoryRuntimeError::Unavailable)?;
        let matches = undo.as_ref().is_some_and(|entry| entry.token == token);
        if matches {
            undo.take();
        }
        Ok(matches)
    }

    pub(crate) fn set_history_prediction_team(
        &self,
        team: TeamDps,
        upper: bool,
    ) -> Result<bool, TeamImportError> {
        let outcome = self.0.team_import.replace_half(upper, team, || {
            self.publish_settings_effects(SettingsMutationEffects::SETTINGS);
        })?;
        Ok(outcome.changed)
    }
}
