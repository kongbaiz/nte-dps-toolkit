use nte_dps_tool::{
    core::{
        diagnostics::{DiagnosticRun, DiagnosticSnapshot},
        live_capture::ReplayStartError,
    },
    engine::{
        capture::{
            CaptureExportNetwork, CaptureExportOptions, CaptureExportPlan,
            PreparedCaptureJsonReplay,
        },
        model::CaptureQualitySummary,
    },
};

use super::*;

impl AppState {
    pub(crate) fn diagnostics_revision(&self) -> Result<(u64, u64, u64), CoreError> {
        let packet_generation = self
            .0
            .live_capture
            .with_packet_state(|revision, _, _| revision.generation)?;
        Ok((
            self.0.live_capture.revision(),
            packet_generation,
            self.0.diagnostics.revision(),
        ))
    }

    pub(crate) fn diagnostics_input(&self) -> Result<DiagnosticSnapshot, CoreError> {
        let config = self.ui_config();
        let status = self.0.live_capture.status();
        let replay_running = self.0.live_capture.replay_running()?;
        let raw_packet_count = self
            .0
            .live_capture
            .raw_capture_snapshot()?
            .map_or(0, |raw| raw.packet_count as usize);
        let (parsed_packet_count, hit_count) = self
            .0
            .live_capture
            .with_state(|state| (state.packet_count, state.hits.len()))?;
        Ok(DiagnosticSnapshot {
            capture_running: matches!(
                status.phase,
                LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
            ) && !replay_running,
            replay_running,
            active_capture_filter: self.0.live_capture.active_capture_filter()?,
            raw_packet_count,
            parsed_packet_count,
            hit_count,
            dropped_history_archives: self.0.live_capture.dropped_history_archives(),
            include_incoming: true,
            server_damage_calibration: config.server_damage_calibration,
            last_diagnostic: status.issue.map(|issue| format!("{issue:?}")),
            manual_capture_device: config.manual_capture_device,
        })
    }

    pub(crate) fn diagnostics_report(&self) -> Option<DiagnosticRun> {
        self.0.diagnostics.snapshot().1
    }

    pub(crate) fn diagnostics_report_snapshot(&self) -> (u64, Option<DiagnosticRun>) {
        self.0.diagnostics.snapshot()
    }

    pub(crate) fn store_diagnostics_report(&self, report: DiagnosticRun) -> bool {
        self.0.diagnostics.store(report)
    }

    pub(crate) fn diagnostics_quality(&self) -> Result<CaptureQualitySummary, CoreError> {
        self.0.live_capture.quality_summary()
    }

    pub(crate) fn diagnostics_raw_capture(
        &self,
    ) -> Result<Option<nte_dps_tool::engine::capture::RawCaptureSnapshot>, CoreError> {
        self.0.live_capture.raw_capture_snapshot()
    }

    pub(crate) fn save_diagnostics_raw_capture(
        &self,
        path: &std::path::Path,
    ) -> Result<Result<(u64, u64), String>, CoreError> {
        self.0.live_capture.save_last_raw_capture(path)
    }

    pub(crate) fn begin_replay_import(
        &self,
        replace_current: bool,
    ) -> Result<ReplayImportReservation, ReplayImportError> {
        self.0
            .replay_import
            .reserve(ReplayImportReservationState::ReplayImport)?;
        let reservation = ReplayImportReservation {
            state: self.clone(),
            active: true,
        };
        let active = matches!(
            self.capture_phase(),
            LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
        ) || self.replay_running()?;
        if active && !replace_current {
            return Err(ReplayImportError::Capture(CoreError::new(
                CoreErrorCode::CaptureAlreadyRunning,
                "capture or replay is already active",
            )));
        }
        if self.session_has_data()? && !replace_current {
            return Err(ReplayImportError::Capture(CoreError::new(
                CoreErrorCode::CaptureAlreadyRunning,
                "replay import requires confirmation before replacing the current session",
            )));
        }
        Ok(reservation)
    }

    pub(super) fn request_diagnostics_replay(
        &self,
        kind: CaptureReplayKind,
        path: PathBuf,
    ) -> Result<(), ReplayImportError> {
        let config = self.ui_config();
        let local_ip_hint = self
            .diagnostics_report()
            .and_then(|run| run.environment.local_ip)
            .and_then(|local_ip| local_ip.parse().ok());
        let result = self
            .0
            .live_capture
            .request_replay(
                kind,
                path,
                local_ip_hint,
                true,
                config.server_damage_calibration,
            )
            .map_err(|error| match error {
                ReplayStartError::Capture(error) => ReplayImportError::Capture(error),
                ReplayStartError::JsonImport(error) => ReplayImportError::JsonImport(error),
            });
        if result.is_ok() {
            self.return_main_presentation_to_live();
        }
        result
    }

    pub(super) fn request_diagnostics_prepared_json_replay(
        &self,
        prepared: PreparedCaptureJsonReplay,
    ) -> Result<(), ReplayImportError> {
        let result = self
            .0
            .live_capture
            .request_prepared_json_replay(prepared)
            .map_err(ReplayImportError::Capture);
        if result.is_ok() {
            self.return_main_presentation_to_live();
        }
        result
    }

    pub(crate) fn diagnostics_capture_export_plan(
        &self,
    ) -> Result<(CaptureExportPlan, u64), CoreError> {
        let config = self.ui_config();
        let dps_time_mode = self.live_dps_time_mode()?;
        let game_network = self
            .diagnostics_report()
            .and_then(|run| run.environment.game_connection)
            .map(|network| CaptureExportNetwork {
                pid: network.pid,
                local_ip: network.local_ip,
                remote_ip: network.remote_ip,
                remote_port: network.remote_port,
            });
        self.0.live_capture.with_packet_state(|revision, _, state| {
            (
                nte_dps_tool::engine::capture::CaptureExportDocument::prepare(
                    state,
                    CaptureExportOptions {
                        filter: config.capture_filter,
                        include_incoming: true,
                        game_network,
                        dps_time_mode: DpsTimeBasis::from_subtract_time_stop(
                            subtract_time_stop_for_state(dps_time_mode, state),
                        ),
                    },
                ),
                revision.session_generation,
            )
        })
    }

    /// Copies one bounded hit page while proving it still belongs to the
    /// immutable export plan. Disk serialization happens after this closure
    /// releases the authoritative state lock.
    pub(crate) fn diagnostics_capture_export_hit_page(
        &self,
        session_generation: u64,
        hits_generation: u64,
        hit_count: usize,
        start: usize,
        limit: usize,
    ) -> Result<Vec<Hit>, CoreError> {
        self.0
            .live_capture
            .with_packet_state(|revision, _, state| {
                let Some(end) = start.checked_add(limit).filter(|end| *end <= hit_count) else {
                    return Err(CoreError::new(
                        CoreErrorCode::CaptureStateUnavailable,
                        "capture export requested an invalid hit page",
                    ));
                };
                if revision.session_generation != session_generation
                    || state.hits_generation != hits_generation
                    || state.hits.len() != hit_count
                {
                    return Err(CoreError::new(
                        CoreErrorCode::CaptureStateUnavailable,
                        "capture export source changed during streaming",
                    ));
                }
                Ok(state.hits.range(start..end).cloned().collect())
            })?
    }

    pub(crate) fn diagnostics_has_exportable_state(&self) -> Result<bool, CoreError> {
        self.0.live_capture.with_state(|state| {
            !state.hits.is_empty() || !state.packets.is_empty() || !state.empty_curtain.is_empty()
        })
    }
}
