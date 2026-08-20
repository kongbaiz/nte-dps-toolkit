use std::io::{self, Write};

use nte_dps_tool::storage::history::{
    HistoryDeleteTombstone, HistoryRecord, HistoryRecordExportError, HistoryRecordLoadError,
    MAX_HISTORY_IMPORT_BYTES, compare_records, export_prepared_history_record_to_path,
    import_record, import_record_json, load_history_record_by_id_with_max_detail_bytes,
    load_history_summaries, prepare_history_record_export, restore_tombstoned_record, save_summary,
    save_summary_with_details, tombstone_record,
};
use nte_dps_tool::storage::i18n;
use tauri::{AppHandle, State, WebviewWindow};

use crate::{
    contract::{
        CommandError,
        history::{
            HistoryComparisonSnapshot, HistoryDeleteSnapshot, HistoryExportSnapshot,
            HistoryFileActionSnapshot, HistoryImportFileSnapshot, HistorySnapshot,
        },
    },
    file_dialog::{self, DialogOutcome},
    history_runtime::history_runtime_unavailable,
    state::AppState,
    team_import_service::TeamImportError,
    windows::{console, island},
};

const MAX_HISTORY_INLINE_EXPORT_BYTES: usize = 8 * 1024 * 1024;

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
        let title = i18n::t("NTE history summary");
        let selection = file_dialog::choose_json_open_path(&window, title)
            .await
            .map_err(|error| {
                log::error!("native History import dialog failed: {error}");
                history_file_dialog_failed()
            })?;
        let state = state.inner().clone();
        tauri::async_runtime::spawn_blocking(move || match selection {
            DialogOutcome::Selected(path) => {
                let imported_record_id = run_history_operation(&state, || {
                    let record = import_record(&path).map_err(history_import_error)?;
                    state.bump_history_revision();
                    Ok(record.id)
                })?;
                Ok(HistoryImportFileSnapshot {
                    performed: true,
                    imported_record_id: Some(imported_record_id),
                    history: load_stable_snapshot(&state)?,
                })
            }
            DialogOutcome::Cancelled => Ok(HistoryImportFileSnapshot {
                performed: false,
                imported_record_id: None,
                history: load_stable_snapshot(&state)?,
            }),
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
    let archive = state
        .prepare_current_history_archive()
        .map_err(CommandError::from_core)?
        .ok_or_else(|| {
            CommandError::history(
                "history_no_current_summary",
                "No combat summary to save; capture first or import a replay",
            )
        })?;
    let state_for_task = state.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        run_history_operation(&state_for_task, || {
            let result = match archive.details {
                Some(details) => save_summary_with_details(archive.summary, details),
                None => save_summary(archive.summary),
            };
            result.map_err(|error| {
                log::warn!("save History record failed: {error}");
                CommandError::history("history_save_failed", "History record could not be saved.")
            })?;
            state_for_task.bump_history_revision();
            Ok(())
        })?;
        load_stable_snapshot(&state_for_task)
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
        run_history_operation(&state, || {
            import_record_json(&json).map_err(history_import_error)?;
            state.bump_history_revision();
            Ok(())
        })?;
        load_stable_snapshot(&state)
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
        let undo_token = run_history_operation(&state, || {
            state
                .ensure_history_undo_available()
                .map_err(history_runtime_unavailable)?;
            // Storage moves the main manifest and every validated sidecar into
            // a same-volume tombstone without hydrating the detail payload.
            let tombstone = tombstone_record(&record_id).map_err(|error| {
                log::warn!("delete History record failed: {error}");
                CommandError::history(
                    "history_delete_failed",
                    "History record could not be deleted.",
                )
            })?
            .ok_or_else(history_record_not_found)?;
            let undo_token = match state.remember_deleted_history_tombstone(tombstone.clone()) {
                Ok(token) => token,
                Err(error) => {
                    if let Err(restore_error) = restore_tombstoned_record(&tombstone) {
                        log::warn!(
                            "History undo registration and deletion rollback failed: {restore_error}"
                        );
                    }
                    return Err(history_runtime_unavailable(error));
                }
            };
            state.bump_history_revision();
            Ok(undo_token)
        })?;
        Ok(HistoryDeleteSnapshot {
            history: load_stable_snapshot(&state)?,
            undo_token,
            undo_expires_ms: crate::state::HISTORY_UNDO_WINDOW.as_millis() as u32,
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
        run_history_operation(&state, || {
            restore_deleted_history_operation(&state, &undo_token, |tombstone| {
                restore_tombstoned_record(tombstone).map_err(|_| {
                    log::warn!("restore History record storage operation did not finish");
                    CommandError::history(
                        "history_restore_failed",
                        "History record could not be restored.",
                    )
                })
            })?;
            state.bump_history_revision();
            Ok(())
        })?;
        load_stable_snapshot(&state)
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
    _state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<HistoryExportSnapshot, CommandError> {
    console::validate_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || prepare_history_export(&record_id))
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
        let state = state.inner().clone();
        let state_for_name = state.clone();
        let record_id_for_name = record_id.clone();
        let file_name = tauri::async_runtime::spawn_blocking(move || {
            history_export_file_name(&state_for_name, &record_id_for_name)
        })
        .await
        .map_err(history_task_failed)??;
        let title = i18n::t("NTE history summary");
        match file_dialog::choose_json_save_path(&window, title, file_name)
            .await
            .map_err(|error| {
                log::error!("native History export dialog failed: {error}");
                history_file_dialog_failed()
            })? {
            DialogOutcome::Selected(path) => tauri::async_runtime::spawn_blocking(move || {
                // Descriptor preparation and chunk I/O intentionally run without
                // the History mutation transaction. The descriptor binds the
                // main manifest identity, while every sidecar is independently
                // bounded and checksum-bound; concurrent delete/prune therefore
                // yields a typed source-change failure and leaves the atomic
                // destination untouched instead of blocking archive persistence.
                let prepared =
                    prepare_history_record_export(&record_id).map_err(history_file_export_error)?;
                export_prepared_history_record_to_path(&prepared, &path)
                    .map_err(history_file_export_error)?;
                Ok(HistoryFileActionSnapshot { performed: true })
            })
            .await
            .map_err(history_task_failed)?,
            DialogOutcome::Cancelled => Ok(HistoryFileActionSnapshot { performed: false }),
        }
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
        let (loaded, _) = load_stable_history_summaries(&state)?;
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
        // Prediction teams are summary-derived; do not hydrate a selected
        // record's potentially multi-gigabyte detail chunks.
        let record = find_summary_record(&state_for_task, &record_id)?;
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
    .await
    .map_err(history_task_failed)??;
    state
        .set_history_prediction_team(team, upper)
        .map_err(team_import_error)?;
    load_snapshot(state).await
}

fn team_import_error(_error: TeamImportError) -> CommandError {
    CommandError::team_import_state_unavailable()
}

async fn load_snapshot(state: AppState) -> Result<HistorySnapshot, CommandError> {
    tauri::async_runtime::spawn_blocking(move || load_stable_snapshot(&state))
        .await
        .map_err(history_task_failed)?
}

fn load_stable_snapshot(state: &AppState) -> Result<HistorySnapshot, CommandError> {
    let (loaded, revision) = load_stable_history_summaries(state)?;
    Ok(project_snapshot(state, loaded, revision))
}

fn load_stable_history_summaries(
    state: &AppState,
) -> Result<(nte_dps_tool::storage::history::HistoryLoadResult, u64), CommandError> {
    for _ in 0..3 {
        let revision = state
            .with_history_transaction(|| state.history_revision())
            .map_err(history_runtime_unavailable)?;
        let loaded = load_history_summaries();
        let current_revision = state
            .with_history_transaction(|| state.history_revision())
            .map_err(history_runtime_unavailable)?;
        if current_revision == revision {
            return Ok((loaded, revision));
        }
    }
    Err(history_runtime_unavailable(
        crate::state::HistoryRuntimeError::Unavailable,
    ))
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

fn find_summary_record(state: &AppState, record_id: &str) -> Result<HistoryRecord, CommandError> {
    load_stable_history_summaries(state)?
        .0
        .records
        .into_iter()
        .find(|record| record.id == record_id)
        .ok_or_else(history_record_not_found)
}

fn history_export_file_name(state: &AppState, record_id: &str) -> Result<String, CommandError> {
    let record = find_summary_record(state, record_id)?;
    Ok(format!(
        "nte_history_{}_{}.json",
        record.file_timestamp(),
        record.id
    ))
}

fn prepare_history_export(record_id: &str) -> Result<HistoryExportSnapshot, CommandError> {
    let record = match load_history_record_by_id_with_max_detail_bytes(
        record_id,
        MAX_HISTORY_INLINE_EXPORT_BYTES as u64,
    ) {
        Ok(Some(record)) => record,
        Ok(None) | Err(HistoryRecordLoadError::InvalidId) => {
            return Err(history_record_not_found());
        }
        Err(
            HistoryRecordLoadError::DetailsTooLarge { .. }
            | HistoryRecordLoadError::DetailHitsTooLarge { .. },
        ) => {
            return Err(history_inline_export_too_large());
        }
        Err(HistoryRecordLoadError::LoadFailed) => {
            return Err(CommandError::history(
                "history_record_load_failed",
                "History record could not be loaded.",
            ));
        }
    };
    let mut writer = BoundedHistoryExportWriter::new(MAX_HISTORY_INLINE_EXPORT_BYTES);
    serde_json::to_writer_pretty(&mut writer, &record).map_err(|error| {
        log::warn!("serialize History record failed: {error}");
        history_inline_export_too_large()
    })?;
    writer
        .write_all(b"\n")
        .map_err(|_| history_inline_export_too_large())?;
    let json = String::from_utf8(writer.into_inner()).map_err(|_| {
        CommandError::history(
            "history_export_failed",
            "History record could not be exported.",
        )
    })?;
    Ok(HistoryExportSnapshot {
        file_name: format!("nte_history_{}_{}.json", record.file_timestamp(), record.id),
        json,
    })
}

fn history_inline_export_too_large() -> CommandError {
    CommandError::history(
        "history_inline_export_too_large",
        "This History record is too large for inline export; use Export File.",
    )
}

fn history_file_export_error(error: HistoryRecordExportError) -> CommandError {
    log::warn!("History file export failed: {error}");
    match error {
        HistoryRecordExportError::InvalidId | HistoryRecordExportError::NotFound => {
            history_record_not_found()
        }
        HistoryRecordExportError::SourceChanged => CommandError::history(
            "history_export_source_changed",
            "History record could not be exported.",
        ),
        HistoryRecordExportError::CorruptRecord => CommandError::history(
            "history_record_load_failed",
            "History record could not be loaded.",
        ),
        HistoryRecordExportError::DestinationWriteFailed => CommandError::history(
            "history_export_failed",
            "History record could not be exported.",
        ),
    }
}

struct BoundedHistoryExportWriter {
    bytes: Vec<u8>,
    max_bytes: usize,
}

impl BoundedHistoryExportWriter {
    fn new(max_bytes: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(max_bytes.min(64 * 1024)),
            max_bytes,
        }
    }

    fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

impl Write for BoundedHistoryExportWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let remaining = self.max_bytes.saturating_sub(self.bytes.len());
        if bytes.len() > remaining {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "inline History export exceeds its byte budget",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn run_history_operation<T>(
    state: &AppState,
    action: impl FnOnce() -> Result<T, CommandError>,
) -> Result<T, CommandError> {
    state
        .with_history_transaction(action)
        .map_err(history_runtime_unavailable)?
}

fn restore_deleted_history_operation(
    state: &AppState,
    undo_token: &str,
    restore: impl FnOnce(&HistoryDeleteTombstone) -> Result<(), CommandError>,
) -> Result<(), CommandError> {
    let tombstone = state
        .peek_deleted_history_tombstone(undo_token)
        .map_err(history_runtime_unavailable)?
        .ok_or_else(|| {
            CommandError::history(
                "history_undo_expired",
                "The deleted history record can no longer be restored.",
            )
        })?;
    restore(&tombstone)?;
    if !state
        .consume_deleted_history(undo_token)
        .map_err(history_runtime_unavailable)?
    {
        return Err(history_runtime_unavailable(
            crate::state::HistoryRuntimeError::Unavailable,
        ));
    }
    Ok(())
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
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn temporary_history_tombstone(tag: &str) -> (PathBuf, HistoryDeleteTombstone) {
        use nte_dps_tool::{
            engine::model::CombatSessionSummary,
            storage::history::{save_summary_to_dir, tombstone_record_from_dir},
        };

        let directory = std::env::temp_dir().join(format!(
            "nte-history-command-undo-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time after epoch")
                .as_nanos()
        ));
        let record = save_summary_to_dir(&directory, CombatSessionSummary::default())
            .expect("save History command fixture");
        let tombstone = tombstone_record_from_dir(&directory, &record.id)
            .expect("tombstone History command fixture")
            .expect("History command fixture exists");
        (directory, tombstone)
    }

    #[test]
    fn inline_export_writer_rejects_overflow_without_partial_append() {
        let mut writer = BoundedHistoryExportWriter::new(4);
        writer.write_all(b"1234").expect("exact budget");
        assert!(writer.write_all(b"5").is_err());
        assert_eq!(writer.into_inner(), b"1234");
    }

    #[test]
    fn file_export_releases_history_transaction_before_chunk_io() {
        let source = include_str!("history.rs");
        let body = source
            .split_once("pub(crate) async fn export_history_record_file(")
            .and_then(|(_, tail)| tail.split_once("\n#[tauri::command]"))
            .map(|(body, _)| body)
            .expect("file export command source should remain discoverable");
        let prepare = body
            .find("prepare_history_record_export")
            .expect("short descriptor preparation");
        let export = body
            .find("export_prepared_history_record_to_path")
            .expect("lock-free chunk export");
        assert!(prepare < export);
        assert_eq!(body.matches("run_history_operation").count(), 0);
        assert!(!body.contains("export_history_record_to_path"));
        assert!(!body.contains("find_record(&record_id)"));
        assert!(
            !body[prepare..].contains("run_history_operation"),
            "descriptor preparation and chunk I/O must not hold the History mutation transaction"
        );
    }

    #[test]
    fn summary_scans_are_not_nested_inside_history_mutation_transactions() {
        let source = include_str!("history.rs");
        assert!(
            !source.contains(
                "run_history_operation(&state, || {\n                Ok(project_snapshot"
            )
        );
        assert!(!source.contains(
            "run_history_operation(&state, || {\n            let loaded = load_history_summaries"
        ));
        assert!(source.contains("fn load_stable_snapshot(state: &AppState)"));
    }

    #[test]
    fn interactive_hit_budget_maps_to_stable_inline_export_error() {
        let error = match (HistoryRecordLoadError::DetailHitsTooLarge {
            count: u64::MAX,
            limit: 1,
        }) {
            HistoryRecordLoadError::DetailsTooLarge { .. }
            | HistoryRecordLoadError::DetailHitsTooLarge { .. } => {
                history_inline_export_too_large()
            }
            _ => unreachable!("typed load fixture"),
        };
        assert_eq!(error.code, "history_inline_export_too_large");
    }

    #[test]
    fn file_export_source_change_has_a_stable_typed_command_error() {
        let error = history_file_export_error(HistoryRecordExportError::SourceChanged);
        assert_eq!(error.code, "history_export_source_changed");
        assert_eq!(error.message_key, "History record could not be exported.");
        assert!(error.message_arguments.is_empty());
    }

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

    #[test]
    fn poisoned_history_runtime_has_a_stable_boundary_error() {
        let error = history_runtime_unavailable(crate::state::HistoryRuntimeError::Unavailable);

        assert_eq!(error.code, "history_runtime_unavailable");
        assert_eq!(error.message_key, "History operation did not finish.");
        assert!(error.message_arguments.is_empty());
        assert_eq!(error.diagnostic_line, None);
    }

    #[test]
    fn failed_restore_keeps_deleted_history_available_for_retry() {
        let state = AppState::default();
        let (directory, tombstone) = temporary_history_tombstone("failed-restore");
        let record_id = tombstone.record_id().to_owned();
        let token = state
            .remember_deleted_history_tombstone(tombstone)
            .expect("remember deleted History record");

        let failed = run_history_operation(&state, || {
            restore_deleted_history_operation(&state, &token, |_| {
                Err(CommandError::history(
                    "history_restore_failed",
                    "History record could not be restored.",
                ))
            })
        });

        assert!(failed.is_err());
        assert_eq!(
            state
                .peek_deleted_history_tombstone(&token)
                .expect("peek History undo after failed restore")
                .expect("failed restore must retain History undo")
                .record_id(),
            record_id
        );
        run_history_operation(&state, || {
            restore_deleted_history_operation(&state, &token, |_| Ok(()))
        })
        .expect("retry deleted History restore");
        assert!(
            state
                .peek_deleted_history_tombstone(&token)
                .expect("peek consumed History undo")
                .is_none()
        );
        let _ = fs::remove_dir_all(directory);
    }
}
