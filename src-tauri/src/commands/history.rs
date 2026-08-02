use nte_dps_tool::storage::history::{
    HistoryRecord, MAX_HISTORY_IMPORT_BYTES, compare_records, delete_record, import_record,
    import_record_json, load_history, restore_record, save_summary, save_summary_with_details,
};
use nte_dps_tool::storage::{i18n, io_util::atomic_write_text};
use tauri::{AppHandle, State, WebviewWindow};

use crate::{
    contract::{
        CommandError,
        history::{
            HistoryComparisonSnapshot, HistoryDeleteSnapshot, HistoryExportSnapshot,
            HistoryFileActionSnapshot, HistoryImportFileSnapshot, HistorySnapshot,
        },
    },
    state::AppState,
    windows::{console, island},
};

#[tauri::command]
pub(crate) async fn get_history_snapshot(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<HistorySnapshot, CommandError> {
    console::validate_window(&window)?;
    load_snapshot(state.inner().clone()).await
}

#[tauri::command]
pub(crate) async fn import_history_record_file(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<HistoryImportFileSnapshot, CommandError> {
    console::validate_window(&window)?;
    #[cfg(windows)]
    {
        use nte_dps_tool::platform::file_dialog::{OpenFileDialogOutcome, choose_json_open_path};

        let owner = window.hwnd().map_err(|_| history_file_dialog_failed())?.0 as isize;
        let title = i18n::t("NTE history summary");
        let state = state.inner().clone();
        tauri::async_runtime::spawn_blocking(move || match choose_json_open_path(owner, &title) {
            Ok(OpenFileDialogOutcome::Selected(path)) => state.with_history_transaction(|| {
                let record = import_record(&path).map_err(history_import_error)?;
                let imported_record_id = record.id;
                let revision = state.bump_history_revision();
                Ok(HistoryImportFileSnapshot {
                    performed: true,
                    imported_record_id: Some(imported_record_id),
                    history: project_snapshot(&state, load_history(), revision),
                })
            }),
            Ok(OpenFileDialogOutcome::Cancelled) => state.with_history_transaction(|| {
                Ok(HistoryImportFileSnapshot {
                    performed: false,
                    imported_record_id: None,
                    history: project_snapshot(&state, load_history(), state.history_revision()),
                })
            }),
            Err(code) => {
                log::error!("native History import dialog failed: {code:#010x}");
                Err(history_file_dialog_failed())
            }
        })
        .await
        .map_err(history_task_failed)?
    }
    #[cfg(not(windows))]
    {
        let _ = state;
        Err(history_file_dialog_failed())
    }
}

#[tauri::command]
pub(crate) async fn save_current_history_summary(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<HistorySnapshot, CommandError> {
    console::validate_window(&window)?;
    let state = state.inner().clone();
    let archive = state.prepare_current_history_archive().ok_or_else(|| {
        CommandError::history(
            "history_no_current_summary",
            "No combat summary to save; capture first or import a replay",
        )
    })?;
    let state_for_task = state.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        state_for_task.with_history_transaction(|| {
            let result = match archive.details {
                Some(details) => save_summary_with_details(archive.summary, details),
                None => save_summary(archive.summary),
            };
            result.map_err(|error| {
                log::warn!("save History record failed: {error}");
                CommandError::history("history_save_failed", "History record could not be saved.")
            })?;
            let revision = state_for_task.bump_history_revision();
            Ok(project_snapshot(&state_for_task, load_history(), revision))
        })
    })
    .await
    .map_err(history_task_failed)??;
    island::publish_notice_best_effort(
        &app,
        &state,
        "success",
        "History summary saved",
        Vec::new(),
    );
    Ok(result)
}

#[tauri::command]
pub(crate) async fn import_history_record_json(
    json: String,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<HistorySnapshot, CommandError> {
    console::validate_window(&window)?;
    if json.len() as u64 > MAX_HISTORY_IMPORT_BYTES {
        return Err(CommandError::history(
            "history_import_too_large",
            "History record exceeds the supported import size.",
        ));
    }
    let state = state.inner().clone();
    let notice_state = state.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        state.with_history_transaction(|| {
            import_record_json(&json).map_err(history_import_error)?;
            let revision = state.bump_history_revision();
            Ok(project_snapshot(&state, load_history(), revision))
        })
    })
    .await
    .map_err(history_task_failed)??;
    island::publish_notice_best_effort(
        &app,
        &notice_state,
        "success",
        "History summary imported",
        Vec::new(),
    );
    Ok(result)
}

#[tauri::command]
pub(crate) async fn delete_history_record(
    record_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<HistoryDeleteSnapshot, CommandError> {
    console::validate_window(&window)?;
    let state = state.inner().clone();
    let notice_state = state.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        state.with_history_transaction(|| {
            let record = find_record(&record_id)?;
            if !delete_record(&record_id).map_err(|error| {
                log::warn!("delete History record failed: {error}");
                CommandError::history(
                    "history_delete_failed",
                    "History record could not be deleted.",
                )
            })? {
                return Err(history_record_not_found());
            }
            let undo_token = state.remember_deleted_history(record);
            let revision = state.bump_history_revision();
            Ok(HistoryDeleteSnapshot {
                history: project_snapshot(&state, load_history(), revision),
                undo_token,
                undo_expires_ms: crate::state::HISTORY_UNDO_WINDOW.as_millis() as u32,
            })
        })
    })
    .await
    .map_err(history_task_failed)??;
    island::publish_notice_best_effort(
        &app,
        &notice_state,
        "success",
        "History summary deleted",
        Vec::new(),
    );
    Ok(result)
}

#[tauri::command]
pub(crate) async fn restore_deleted_history_record(
    undo_token: String,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<HistorySnapshot, CommandError> {
    console::validate_window(&window)?;
    let state = state.inner().clone();
    let notice_state = state.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        state.with_history_transaction(|| {
            let record = state.take_deleted_history(&undo_token).ok_or_else(|| {
                CommandError::history(
                    "history_undo_expired",
                    "The deleted history record can no longer be restored.",
                )
            })?;
            restore_record(&record).map_err(|error| {
                log::warn!("restore History record failed: {error}");
                CommandError::history(
                    "history_restore_failed",
                    "History record could not be restored.",
                )
            })?;
            let revision = state.bump_history_revision();
            Ok(project_snapshot(&state, load_history(), revision))
        })
    })
    .await
    .map_err(history_task_failed)??;
    island::publish_notice_best_effort(
        &app,
        &notice_state,
        "success",
        "History summary restored",
        Vec::new(),
    );
    Ok(result)
}

#[tauri::command]
pub(crate) async fn export_history_record_json(
    record_id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<HistoryExportSnapshot, CommandError> {
    console::validate_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_history_transaction(|| prepare_history_export(&record_id))
    })
    .await
    .map_err(history_task_failed)?
}

#[tauri::command]
pub(crate) async fn export_history_record_file(
    record_id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<HistoryFileActionSnapshot, CommandError> {
    console::validate_window(&window)?;
    #[cfg(windows)]
    {
        use nte_dps_tool::platform::file_dialog::{SaveFileDialogOutcome, choose_json_save_path};

        let owner = window.hwnd().map_err(|_| history_file_dialog_failed())?.0 as isize;
        let state = state.inner().clone();
        let export = tauri::async_runtime::spawn_blocking(move || {
            state.with_history_transaction(|| prepare_history_export(&record_id))
        })
        .await
        .map_err(history_task_failed)??;
        let title = i18n::t("NTE history summary");
        tauri::async_runtime::spawn_blocking(move || {
            match choose_json_save_path(owner, &title, &export.file_name) {
                Ok(SaveFileDialogOutcome::Selected(path)) => {
                    atomic_write_text(&path, &export.json).map_err(|error| {
                        log::warn!("write History export failed: {error}");
                        CommandError::history(
                            "history_export_failed",
                            "History record could not be exported.",
                        )
                    })?;
                    Ok(HistoryFileActionSnapshot { performed: true })
                }
                Ok(SaveFileDialogOutcome::Cancelled) => {
                    Ok(HistoryFileActionSnapshot { performed: false })
                }
                Err(code) => {
                    log::error!("native History export dialog failed: {code:#010x}");
                    Err(history_file_dialog_failed())
                }
            }
        })
        .await
        .map_err(history_task_failed)?
    }
    #[cfg(not(windows))]
    {
        let _ = (record_id, state);
        Err(history_file_dialog_failed())
    }
}

#[tauri::command]
pub(crate) async fn compare_history_records(
    left_id: String,
    right_id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<HistoryComparisonSnapshot, CommandError> {
    console::validate_window(&window)?;
    if left_id == right_id {
        return Err(CommandError::history(
            "history_compare_invalid",
            "Select two different history records to compare.",
        ));
    }
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_history_transaction(|| {
            let loaded = load_history();
            let left = loaded
                .records
                .iter()
                .find(|record| record.id == left_id)
                .ok_or_else(history_record_not_found)?;
            let right = loaded
                .records
                .iter()
                .find(|record| record.id == right_id)
                .ok_or_else(history_record_not_found)?;
            Ok(HistoryComparisonSnapshot::from_records(
                compare_records(left, right),
                left,
                right,
                &state.live_capture_resources().characters,
            ))
        })
    })
    .await
    .map_err(history_task_failed)?
}

#[tauri::command]
pub(crate) async fn set_history_prediction_team(
    record_id: String,
    line: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<HistorySnapshot, CommandError> {
    console::validate_window(&window)?;
    let upper = match line.as_str() {
        "upper" => true,
        "lower" => false,
        _ => {
            return Err(CommandError::history(
                "history_prediction_line_invalid",
                "History prediction line is invalid.",
            ));
        }
    };
    let state = state.inner().clone();
    let state_for_task = state.clone();
    let team = tauri::async_runtime::spawn_blocking(move || {
        state_for_task.with_history_transaction(|| {
            let record = find_record(&record_id)?;
            let team = if upper {
                record.upper_team_dps()
            } else {
                record.lower_team_dps()
            };
            team.ok_or_else(|| {
                CommandError::history(
                    "history_prediction_unavailable",
                    "History record has no team usable for this prediction line.",
                )
            })
        })
    })
    .await
    .map_err(history_task_failed)??;
    state.set_history_prediction_team(team, upper);
    load_snapshot(state).await
}

async fn load_snapshot(state: AppState) -> Result<HistorySnapshot, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        state.with_history_transaction(|| {
            project_snapshot(&state, load_history(), state.history_revision())
        })
    })
    .await
    .map_err(history_task_failed)
}

fn project_snapshot(
    state: &AppState,
    loaded: nte_dps_tool::storage::history::HistoryLoadResult,
    revision: u64,
) -> HistorySnapshot {
    HistorySnapshot::from_localized_load(
        loaded,
        revision,
        &state.live_capture_resources().characters,
    )
}

fn find_record(record_id: &str) -> Result<HistoryRecord, CommandError> {
    load_history()
        .records
        .into_iter()
        .find(|record| record.id == record_id)
        .ok_or_else(history_record_not_found)
}

fn prepare_history_export(record_id: &str) -> Result<HistoryExportSnapshot, CommandError> {
    let record = find_record(record_id)?;
    let mut json = serde_json::to_string_pretty(&record).map_err(|error| {
        log::warn!("serialize History record failed: {error}");
        CommandError::history(
            "history_export_failed",
            "History record could not be exported.",
        )
    })?;
    json.push('\n');
    Ok(HistoryExportSnapshot {
        file_name: format!("nte_history_{}_{}.json", record.file_timestamp(), record.id),
        json,
    })
}

fn history_import_error(error: String) -> CommandError {
    log::warn!("import History record failed: {error}");
    if error == "History record exceeds the supported file size" {
        CommandError::history(
            "history_import_too_large",
            "History record exceeds the supported import size.",
        )
    } else {
        CommandError::history(
            "history_import_failed",
            "History record JSON is invalid or unsupported.",
        )
    }
}

fn history_file_dialog_failed() -> CommandError {
    CommandError::history("history_file_dialog_failed", "History file dialog failed.")
}

fn history_record_not_found() -> CommandError {
    CommandError::history("history_record_not_found", "History record was not found.")
}

fn history_task_failed(error: tauri::Error) -> CommandError {
    log::warn!("History background task failed: {error}");
    CommandError::history("history_task_failed", "History operation did not finish.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_import_keeps_size_and_parse_errors_stable() {
        let oversized =
            history_import_error("History record exceeds the supported file size".to_owned());
        assert_eq!(oversized.code, "history_import_too_large");
        assert_eq!(
            oversized.message_key,
            "History record exceeds the supported import size."
        );

        let invalid = history_import_error("invalid JSON".to_owned());
        assert_eq!(invalid.code, "history_import_failed");
        assert_eq!(
            invalid.message_key,
            "History record JSON is invalid or unsupported."
        );
    }
}
