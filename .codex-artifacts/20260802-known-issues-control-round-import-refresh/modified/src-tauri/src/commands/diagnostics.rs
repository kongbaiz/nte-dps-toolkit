use std::time::{SystemTime, UNIX_EPOCH};

use nte_dps_tool::{
    core::{
        CoreError, CoreErrorCode,
        diagnostics::run_capture_diagnostics,
        live_capture::{CaptureReplayKind, LiveCapturePhase},
    },
    engine::capture::write_capture_export,
    storage::i18n,
};
use tauri::{State, WebviewWindow};

use crate::{
    contract::{
        CommandError,
        diagnostics::{DiagnosticsActionResult, DiagnosticsSnapshot},
    },
    state::AppState,
    windows::console,
};

#[tauri::command]
pub(crate) fn get_diagnostics_snapshot(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<DiagnosticsSnapshot, CommandError> {
    console::validate_window(&window)?;
    Ok(snapshot(state.inner()))
}

#[tauri::command]
pub(crate) async fn run_diagnostics(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<DiagnosticsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let report = run_capture_diagnostics(state.diagnostics_input());
        state.store_diagnostics_report(report);
        snapshot(&state)
    })
    .await
    .map_err(|_| operation_error())
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
    if !state.diagnostics_has_exportable_state() {
        return Err(CommandError::diagnostics(
            "no_capture_info",
            "No capture info to export",
        ));
    }

    #[cfg(windows)]
    {
        use nte_dps_tool::platform::file_dialog::{SaveFileDialogOutcome, choose_json_save_path};

        let owner = window.hwnd().map_err(|_| file_dialog_error())?.0 as isize;
        let title = i18n::t("Capture info JSON");
        let default_file_name = format!("nte_capture_{}.json", timestamp_suffix());
        let state = state.inner().clone();
        tauri::async_runtime::spawn_blocking(move || {
            match choose_json_save_path(owner, &title, &default_file_name) {
                Ok(SaveFileDialogOutcome::Selected(path)) => {
                    ensure_inactive(&state)?;
                    let document = state.diagnostics_capture_export();
                    write_capture_export(&path, &document).map_err(|error| {
                        log::error!("write diagnostics capture export failed: {error}");
                        CommandError::diagnostics(
                            "capture_export_failed",
                            "Failed to export capture info",
                        )
                    })?;
                    Ok(DiagnosticsActionResult {
                        performed: true,
                        snapshot: snapshot(&state),
                    })
                }
                Ok(SaveFileDialogOutcome::Cancelled) => Ok(DiagnosticsActionResult {
                    performed: false,
                    snapshot: snapshot(&state),
                }),
                Err(code) => {
                    log::error!("native capture JSON export dialog failed: {code:#010x}");
                    Err(file_dialog_error())
                }
            }
        })
        .await
        .map_err(|_| operation_error())?
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
        use nte_dps_tool::platform::file_dialog::{SaveFileDialogOutcome, choose_pcapng_save_path};

        let owner = window.hwnd().map_err(|_| file_dialog_error())?.0 as isize;
        let title = i18n::t("Full raw capture");
        let default_file_name = format!("nte_raw_{}.pcapng", timestamp_suffix());
        let state = state.inner().clone();
        tauri::async_runtime::spawn_blocking(move || {
            match choose_pcapng_save_path(owner, &title, &default_file_name) {
                Ok(SaveFileDialogOutcome::Selected(path)) => {
                    ensure_inactive(&state)?;
                    state.save_diagnostics_raw_capture(&path).map_err(|error| {
                        log::error!("save diagnostics raw capture failed: {error}");
                        CommandError::diagnostics(
                            "raw_capture_export_failed",
                            "Failed to save the full capture",
                        )
                    })?;
                    Ok(DiagnosticsActionResult {
                        performed: true,
                        snapshot: snapshot(&state),
                    })
                }
                Ok(SaveFileDialogOutcome::Cancelled) => Ok(DiagnosticsActionResult {
                    performed: false,
                    snapshot: snapshot(&state),
                }),
                Err(code) => {
                    log::error!("native PCAPNG export dialog failed: {code:#010x}");
                    Err(file_dialog_error())
                }
            }
        })
        .await
        .map_err(|_| operation_error())?
    }

    #[cfg(not(windows))]
    {
        let _ = state;
        Err(file_dialog_error())
    }
}

pub(crate) fn snapshot(state: &AppState) -> DiagnosticsSnapshot {
    DiagnosticsSnapshot::from_state(state)
}

async fn import_capture(
    state: State<'_, AppState>,
    window: WebviewWindow,
    kind: CaptureReplayKind,
) -> Result<DiagnosticsActionResult, CommandError> {
    console::validate_window(&window)?;
    let reservation = state.begin_replay_import().map_err(capture_error)?;

    #[cfg(windows)]
    {
        use nte_dps_tool::platform::file_dialog::{
            OpenFileDialogOutcome, choose_json_open_path, choose_pcapng_open_path,
        };

        let owner = window.hwnd().map_err(|_| file_dialog_error())?.0 as isize;
        let title = i18n::t(match kind {
            CaptureReplayKind::Pcapng => "Wireshark capture",
            CaptureReplayKind::Json => "NTE exported capture",
        });
        let state = state.inner().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let selection = match kind {
                CaptureReplayKind::Pcapng => choose_pcapng_open_path(owner, &title),
                CaptureReplayKind::Json => choose_json_open_path(owner, &title),
            };
            match selection {
                Ok(OpenFileDialogOutcome::Selected(path)) => {
                    reservation.start(kind, path).map_err(capture_error)?;
                    Ok(DiagnosticsActionResult {
                        performed: true,
                        snapshot: snapshot(&state),
                    })
                }
                Ok(OpenFileDialogOutcome::Cancelled) => Ok(DiagnosticsActionResult {
                    performed: false,
                    snapshot: snapshot(&state),
                }),
                Err(code) => {
                    log::error!("native diagnostics import dialog failed: {code:#010x}");
                    Err(file_dialog_error())
                }
            }
        })
        .await
        .map_err(|_| operation_error())?
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
    ) || state.diagnostics_input().replay_running
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
    };
    CommandError::diagnostics("capture_operation_failed", message_key)
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
}
