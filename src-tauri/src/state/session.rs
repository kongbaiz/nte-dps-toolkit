use nte_dps_tool::core::capture::{
    CaptureControllerOptions, CaptureDeviceSelector, CaptureProfile, RawCaptureMode,
};

use super::*;

impl AppState {
    pub(crate) fn request_capture_start(&self, replace_current: bool) -> Result<(), CoreError> {
        self.0
            .replay_import
            .reserve(ReplayImportReservationState::CaptureStart)
            .map_err(ReplayImportError::into_core)?;
        let result = (|| {
            if self.session_has_data()? && !replace_current {
                return Err(CoreError::new(
                    CoreErrorCode::CaptureAlreadyRunning,
                    "starting capture requires confirmation before replacing the current session",
                ));
            }
            let config = self.ui_config();
            if config.data_mode == nte_dps_tool::core::toolkit::DataMode::Plugin {
                let pid = nte_dps_tool::platform::network::game_process_id()
                    .map_err(|_| {
                        CoreError::new(
                            CoreErrorCode::SystemProbeFailed,
                            "plugin process probe failed",
                        )
                    })?
                    .ok_or_else(|| {
                        CoreError::new(
                            CoreErrorCode::GameProcessNotFound,
                            "plugin host not running",
                        )
                    })?;
                return self.0.live_capture.request_plugin_start(pid);
            }
            let device = config
                .manual_capture_device
                .clone()
                .map_or(CaptureDeviceSelector::Auto, CaptureDeviceSelector::Name);
            self.0.live_capture.request_start(CaptureControllerOptions {
                profile: CaptureProfile::Combat,
                device,
                filter: config.capture_filter.clone(),
                include_incoming: true,
                server_damage_calibration: config.server_damage_calibration,
                raw_capture: RawCaptureMode::Enabled,
                raw_capture_directory: capture_log_dir(),
                expose_raw_capture_path: false,
                packet_emission: DESKTOP_PACKET_EMISSION_MODE,
            })
        })();
        self.0
            .replay_import
            .release(ReplayImportReservationState::CaptureStart)
            .map_err(ReplayImportError::into_core)?;
        if result.is_ok() {
            self.return_main_presentation_to_live();
        }
        result
    }

    pub(crate) fn request_capture_stop(&self) -> Result<(), CoreError> {
        self.0.live_capture.request_stop()
    }

    pub(crate) fn stop_active_capture_and_wait(&self, timeout: Duration) -> Result<(), CoreError> {
        if matches!(
            self.capture_phase(),
            LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Failed
        ) || self.replay_running()?
        {
            self.request_capture_stop()?;
        }
        let deadline = Instant::now() + timeout;
        while matches!(
            self.capture_phase(),
            LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
        ) || self.replay_running()?
        {
            if Instant::now() >= deadline {
                return Err(CoreError::new(
                    CoreErrorCode::SystemProbeFailed,
                    "capture or replay did not stop before the replacement timeout",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    pub(crate) fn capture_phase(&self) -> LiveCapturePhase {
        self.0.live_capture.status().phase
    }

    pub(crate) fn session_has_data(&self) -> Result<bool, CoreError> {
        self.0.live_capture.with_state(|state| {
            !state.hits.is_empty()
                || !state.packets.is_empty()
                || !state.stats.is_empty()
                || !state.empty_curtain.is_empty()
                || state.abyss.is_active()
        })
    }

    pub(crate) fn ensure_session_undo_runtime_available(&self) -> Result<(), SessionUndoError> {
        self.0.session_undo.ensure_available()
    }

    pub(crate) fn reset_session_with_undo_action(
        &self,
    ) -> Result<Option<String>, SessionUndoError> {
        self.0.session_undo.ensure_available()?;
        let (previous, quality_source) = self
            .0
            .live_capture
            .state_and_source_snapshot()
            .map_err(|_| SessionUndoError::StateUnavailable)?;
        let has_data = !previous.hits.is_empty()
            || !previous.packets.is_empty()
            || !previous.stats.is_empty()
            || !previous.empty_curtain.is_empty()
            || previous.abyss.is_active();
        if !has_data {
            self.0
                .live_capture
                .reset_session()
                .map_err(|_| SessionUndoError::StateUnavailable)?;
            self.return_main_presentation_to_live();
            return Ok(None);
        }
        let token = self.0.session_undo.remember(previous, quality_source)?;
        if self.0.live_capture.reset_session().is_err() {
            self.0.session_undo.consume_matching(&token)?;
            return Err(SessionUndoError::StateUnavailable);
        }
        self.return_main_presentation_to_live();
        Ok(Some(token))
    }

    pub(crate) fn reset_session_with_undo(&self) -> Result<Option<String>, CoreError> {
        self.reset_session_with_undo_action()
            .map_err(SessionUndoError::into_core)
    }

    pub(crate) fn clear_session_action(&self) -> Result<(), SessionUndoError> {
        self.0.session_undo.ensure_available()?;
        self.0
            .live_capture
            .reset_session()
            .map_err(|_| SessionUndoError::StateUnavailable)?;
        self.0.session_undo.clear()?;
        self.return_main_presentation_to_live();
        Ok(())
    }

    pub(crate) fn clear_session(&self) -> Result<(), CoreError> {
        self.clear_session_action()
            .map_err(SessionUndoError::into_core)
    }

    pub(crate) fn undo_session_reset(&self, token: &str) -> Result<(), SessionUndoError> {
        self.0.session_undo.ensure_available()?;
        if matches!(
            self.capture_phase(),
            LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
        ) || self
            .replay_running()
            .map_err(|_| SessionUndoError::StateUnavailable)?
        {
            return Err(SessionUndoError::Busy);
        }
        if self
            .session_has_data()
            .map_err(|_| SessionUndoError::StateUnavailable)?
        {
            return Err(SessionUndoError::NewData);
        }
        let entry = self.0.session_undo.entry_for_restore(token)?;
        self.restore_session_undo_entry(&entry, |state, source| {
            self.0.live_capture.restore_session(state, source)
        })?;
        if !self.0.session_undo.consume_matching(token)? {
            return Err(SessionUndoError::RuntimeUnavailable);
        }
        Ok(())
    }

    pub(super) fn restore_session_undo_entry(
        &self,
        entry: &SessionUndoEntry,
        restore: impl FnOnce(CombatState, CaptureQualitySource) -> Result<(), CoreError>,
    ) -> Result<(), SessionUndoError> {
        let restore_state = entry.state.clone();
        if restore(restore_state, entry.quality_source).is_err() {
            return Err(SessionUndoError::StateUnavailable);
        }
        Ok(())
    }

    pub(crate) fn live_capture_resources(&self) -> LiveCaptureResources {
        self.0.live_capture.resources()
    }

    /// A finite IPC action reserves mode/start changes without holding any hot
    /// mutex during foreign I/O. Dropping it releases the reservation on errors.
    pub(crate) fn reserve_plugin_control(&self) -> Result<PluginControlReservation, CoreError> {
        self.0
            .replay_import
            .reserve(ReplayImportReservationState::PluginControl)
            .map_err(ReplayImportError::into_core)?;
        Ok(PluginControlReservation(self.clone()))
    }

    pub(crate) fn data_mode(&self) -> nte_dps_tool::core::toolkit::DataMode {
        self.ui_config().data_mode
    }

    pub(crate) fn set_data_mode(
        &self,
        mode: nte_dps_tool::core::toolkit::DataMode,
    ) -> Result<(), CoreError> {
        self.0
            .replay_import
            .reserve(ReplayImportReservationState::CaptureStart)
            .map_err(ReplayImportError::into_core)?;
        let result = (|| {
            if self.data_mode() == mode {
                return Ok(());
            }
            self.stop_active_capture_and_wait(Duration::from_secs(15))?;
            self.update_ui_config(|config| {
                config.data_mode = mode;
            })
            .map_err(|_| {
                CoreError::new(
                    CoreErrorCode::SystemProbeFailed,
                    "mode settings save failed",
                )
            })?;
            self.0
                .live_capture
                .set_history_archive_policy(history_archive_policy(&self.ui_config()));
            Ok(())
        })();
        self.0
            .replay_import
            .release(ReplayImportReservationState::CaptureStart)
            .map_err(ReplayImportError::into_core)?;
        result
    }
}
