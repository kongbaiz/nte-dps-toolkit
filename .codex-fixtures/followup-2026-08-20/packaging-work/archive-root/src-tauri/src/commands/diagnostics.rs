use std::time::{SystemTime, UNIX_EPOCH};

use nte_dps_tool::{
    core::{
        CoreError, CoreErrorCode,
        diagnostics::run_capture_diagnostics,
        live_capture::{CaptureReplayKind, LiveCapturePhase},
    },
    engine::capture::write_capture_export_streaming,
    storage::i18n,
};
use tauri::{State, WebviewWindow};

use crate::{
    contract::{
        CommandError,
        diagnostics::{DiagnosticsActionResult, DiagnosticsSnapshot},
    },
    file_dialog::{self, DialogOutcome},
    state::{AppState, ReplayImportError},
    windows::console,
};

#[tauri::command]
pub(crate) fn get_diagnostics_snapshot(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<DiagnosticsSnapshot, CommandError> {
    console::validate_window(&window)?;
    snapshot(state.inner())
}

#[tauri::command]
pub(crate) async fn run_diagnostics(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<DiagnosticsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let report =
            run_capture_diagnostics(state.diagnostics_input().map_err(CommandError::from_core)?);
        state.store_diagnostics_report(report);
        snapshot(&state)
    })
    .await
    .map_err(|_| operation_error())?
}

#[tauri::command]
pub(crate) async fn import_diagnostics_pcapng(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<DiagnosticsActionResult, CommandError> {
    import_capture(state, window, CaptureReplayKind::Pcapng).await
}

#[tauri::command]
pub(crate) async fn import_diagnostics_json(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<DiagnosticsActionResult, CommandError> {
    import_capture(state, window, CaptureReplayKind::Json).await
}

#[tauri::command]
pub(crate) async fn export_diagnostics_json(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<DiagnosticsActionResult, CommandError> {
    console::validate_window(&window)?;
    ensure_inactive(state.inner())?;
    if !state
        .diagnostics_has_exportable_state()
        .map_err(CommandError::from_core)?
    {
        return Err(CommandError::diagnostics(
            "no_capture_info",
            "No capture info to export",
        ));
    }

    #[cfg(windows)]
    {
        let title = i18n::t("Capture info JSON");
        let default_file_name = format!("nte_capture_{}.json", timestamp_suffix());
        let state = state.inner().clone();
        match file_dialog::choose_json_save_path(&window, title, default_file_name)
            .await
            .map_err(|error| {
                log::error!("native capture JSON export dialog failed: {error}");
                file_dialog_error()
            })? {
            DialogOutcome::Selected(path) => tauri::async_runtime::spawn_blocking(move || {
                ensure_inactive(&state)?;
                let (plan, session_generation) = state
                    .diagnostics_capture_export_plan()
                    .map_err(CommandError::from_core)?;
                let hits_generation = plan.hits_generation();
                let hit_count = plan.hit_count();
                write_capture_export_streaming(&path, &plan, |start, limit| {
                    state
                        .diagnostics_capture_export_hit_page(
                            session_generation,
                            hits_generation,
                            hit_count,
                            start,
                            limit,
                        )
                        .map_err(|error| error.detail)
                })
                .map_err(|error| {
                    log::error!("write diagnostics capture export failed: {error}");
                    CommandError::diagnostics(
                        "capture_export_failed",
                        "Failed to export capture info",
                    )
                })?;
                Ok(DiagnosticsActionResult {
                    performed: true,
                    snapshot: snapshot(&state)?,
                })
            })
            .await
            .map_err(|_| operation_error())?,
            DialogOutcome::Cancelled => Ok(DiagnosticsActionResult {
                performed: false,
                snapshot: snapshot(&state)?,
            }),
        }
    }

    #[cfg(not(windows))]
    {
        let _ = state;
        Err(file_dialog_error())
    }
}

#[tauri::command]
pub(crate) async fn export_diagnostics_pcapng(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<DiagnosticsActionResult, CommandError> {
    console::validate_window(&window)?;
    ensure_inactive(state.inner())?;

    #[cfg(windows)]
    {
        let title = i18n::t("Full raw capture");
        let default_file_name = format!("nte_raw_{}.pcapng", timestamp_suffix());
        let state = state.inner().clone();
        match file_dialog::choose_pcapng_save_path(&window, title, default_file_name)
            .await
            .map_err(|error| {
                log::error!("native PCAPNG export dialog failed: {error}");
                file_dialog_error()
            })? {
            DialogOutcome::Selected(path) => tauri::async_runtime::spawn_blocking(move || {
                ensure_inactive(&state)?;
                state
                    .save_diagnostics_raw_capture(&path)
                    .map_err(CommandError::from_core)?
                    .map_err(|_| {
                        log::error!("save diagnostics raw capture did not finish");
                        CommandError::diagnostics(
                            "raw_capture_export_failed",
                            "Failed to save the full capture",
                        )
                    })?;
                Ok(DiagnosticsActionResult {
                    performed: true,
                    snapshot: snapshot(&state)?,
                })
            })
            .await
            .map_err(|_| operation_error())?,
            DialogOutcome::Cancelled => Ok(DiagnosticsActionResult {
                performed: false,
                snapshot: snapshot(&state)?,
            }),
        }
    }

    #[cfg(not(windows))]
    {
        let _ = state;
        Err(file_dialog_error())
    }
}

pub(crate) fn snapshot(state: &AppState) -> Result<DiagnosticsSnapshot, CommandError> {
    DiagnosticsSnapshot::from_state(state).map_err(CommandError::from_core)
}

async fn import_capture(
    state: State<'_, AppState>,
    window: WebviewWindow,
    kind: CaptureReplayKind,
) -> Result<DiagnosticsActionResult, CommandError> {
    console::validate_window(&window)?;
    let reservation = state
        .begin_replay_import(false)
        .map_err(replay_import_error)?;

    #[cfg(windows)]
    {
        let title = i18n::t(match kind {
            CaptureReplayKind::Pcapng => "Wireshark capture",
            CaptureReplayKind::Json => "NTE exported capture",
        });
        let selection = match kind {
            CaptureReplayKind::Pcapng => file_dialog::choose_pcapng_open_path(&window, title).await,
            CaptureReplayKind::Json => file_dialog::choose_json_open_path(&window, title).await,
        }
        .map_err(|error| {
            log::error!("native diagnostics import dialog failed: {error}");
            file_dialog_error()
        })?;
        let state = state.inner().clone();
        match selection {
            DialogOutcome::Selected(path) => tauri::async_runtime::spawn_blocking(move || {
                reservation
                    .start(kind, path, false)
                    .map_err(replay_import_error)?;
                Ok(DiagnosticsActionResult {
                    performed: true,
                    snapshot: snapshot(&state)?,
                })
            })
            .await
            .map_err(|_| operation_error())?,
            DialogOutcome::Cancelled => Ok(DiagnosticsActionResult {
                performed: false,
                snapshot: snapshot(&state)?,
            }),
        }
    }

    #[cfg(not(windows))]
    {
        let _ = (state, kind, reservation);
        Err(file_dialog_error())
    }
}

fn ensure_inactive(state: &AppState) -> Result<(), CommandError> {
    if matches!(
        state.capture_phase(),
        LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
    ) || state
        .diagnostics_input()
        .map_err(CommandError::from_core)?
        .replay_running
    {
        Err(CommandError::diagnostics(
            "capture_busy",
            "Stop capture or replay first, then use diagnostics file actions",
        ))
    } else {
        Ok(())
    }
}

fn capture_error(error: CoreError) -> CommandError {
    let message_key = match error.code {
        CoreErrorCode::CaptureAlreadyRunning => {
            "Stop capture or replay first, then use diagnostics file actions"
        }
        CoreErrorCode::NpcapNotFound => "Npcap device enumeration failed",
        CoreErrorCode::GameProcessNotFound => "No active HTGame.exe connection detected",
        CoreErrorCode::CaptureDeviceNotFound => "The selected capture device is unavailable",
        CoreErrorCode::SystemProbeFailed => "Failed to start capture replay",
        CoreErrorCode::CaptureNotRunning => "No live capture task right now",
        CoreErrorCode::CaptureStateUnavailable => {
            return CommandError::from_core(error);
        }
    };
    CommandError::diagnostics("capture_operation_failed", message_key)
}

fn replay_import_error(error: ReplayImportError) -> CommandError {
    match error {
        ReplayImportError::RuntimeUnavailable => CommandError::replay_import_runtime_unavailable(),
        ReplayImportError::Capture(error) => capture_error(error),
        ReplayImportError::JsonImport(error) => {
            CommandError::diagnostics(error.stable_code(), "Failed to start capture replay")
        }
    }
}

fn file_dialog_error() -> CommandError {
    CommandError::diagnostics("file_dialog_failed", "The native file dialog failed")
}

fn operation_error() -> CommandError {
    CommandError::diagnostics(
        "diagnostics_operation_failed",
        "The diagnostics operation did not complete",
    )
}

fn timestamp_suffix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_error_mapping_keeps_private_details_out_of_the_contract() {
        let error = capture_error(CoreError::new(
            CoreErrorCode::CaptureDeviceNotFound,
            r#"\Device\NPF_{private}"#,
        ));

        assert_eq!(error.code, "capture_operation_failed");
        assert_eq!(
            error.message_key,
            "The selected capture device is unavailable"
        );
        assert!(error.message_arguments.is_empty());
    }

    #[test]
    fn replay_runtime_poison_uses_the_shared_stable_error() {
        let error = replay_import_error(ReplayImportError::RuntimeUnavailable);

        assert_eq!(error.code, "replay_import_runtime_unavailable");
        assert_eq!(error.message_key, "Replay import did not complete");
        assert!(error.message_arguments.is_empty());
    }

    #[test]
    fn json_replay_boundary_error_keeps_its_stable_code() {
        let error = replay_import_error(ReplayImportError::JsonImport(
            nte_dps_tool::engine::capture::CaptureImportError::TooLarge { size: 9, limit: 8 },
        ));

        assert_eq!(error.code, "replay_file_too_large");
        assert_eq!(error.message_key, "Failed to start capture replay");
        assert!(error.message_arguments.is_empty());
    }
}
