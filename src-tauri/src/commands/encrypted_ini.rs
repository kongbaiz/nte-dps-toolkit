use nte_dps_tool::{
    core::encrypted_ini::{EncryptedIniError, EncryptedIniSaveOutcome},
    storage::i18n,
};
use tauri::{AppHandle, State, WebviewWindow};

use crate::{
    contract::{
        CommandError,
        encrypted_ini::{
            EncryptedIniSnapshot, OpenEncryptedIniResult, SaveEncryptedIniRequest,
            SaveEncryptedIniResult, parse_encrypted_ini_key, parse_generation,
        },
    },
    encrypted_ini_service::EncryptedIniServiceError,
    file_dialog::{self, DialogOutcome},
    state::AppState,
    windows::{console, island},
};

#[tauri::command]
pub(crate) fn get_encrypted_ini_snapshot(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<EncryptedIniSnapshot, CommandError> {
    console::validate_window(&window)?;
    state
        .encrypted_ini_snapshot()
        .map(EncryptedIniSnapshot::from)
        .map_err(encrypted_ini_error)
}

#[tauri::command]
pub(crate) async fn open_encrypted_ini(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<OpenEncryptedIniResult, CommandError> {
    console::validate_window(&window)?;

    #[cfg(windows)]
    {
        let title = i18n::t("Open Encrypted INI");
        let state = state.inner().clone();
        let notice_state = state.clone();
        let selection = file_dialog::choose_ini_open_path(&window, title)
            .await
            .map_err(|error| {
                log::error!("native encrypted INI open dialog failed: {error}");
                file_dialog_error()
            })?;
        let result = match selection {
            DialogOutcome::Selected(path) => tauri::async_runtime::spawn_blocking(move || {
                state
                    .open_encrypted_ini(path)
                    .map(|snapshot| OpenEncryptedIniResult {
                        opened: true,
                        snapshot: snapshot.into(),
                    })
                    .map_err(encrypted_ini_error)
            })
            .await
            .map_err(|_| operation_error())??,
            DialogOutcome::Cancelled => OpenEncryptedIniResult {
                opened: false,
                snapshot: state
                    .encrypted_ini_snapshot()
                    .map_err(encrypted_ini_error)?
                    .into(),
            },
        };
        if result.opened {
            island::publish_notice_best_effort(
                &app,
                &notice_state,
                "success",
                "Encrypted INI opened",
                Vec::new(),
            );
        }
        Ok(result)
    }

    #[cfg(not(windows))]
    {
        let _ = state;
        Err(file_dialog_error())
    }
}

#[tauri::command]
pub(crate) async fn reload_encrypted_ini(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<EncryptedIniSnapshot, CommandError> {
    console::validate_window(&window)?;
    let state = state.inner().clone();
    let notice_state = state.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        state
            .reload_encrypted_ini()
            .map(EncryptedIniSnapshot::from)
            .map_err(encrypted_ini_error)
    })
    .await
    .map_err(|_| operation_error())??;
    island::publish_notice_best_effort(
        &app,
        &notice_state,
        "success",
        "Encrypted INI reloaded",
        Vec::new(),
    );
    Ok(result)
}

#[tauri::command]
pub(crate) async fn save_encrypted_ini(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
    request: SaveEncryptedIniRequest,
) -> Result<SaveEncryptedIniResult, CommandError> {
    console::validate_window(&window)?;
    let expected_generation =
        parse_generation(&request.expected_generation).ok_or_else(invalid_request_error)?;
    let key = parse_encrypted_ini_key(&request.key).ok_or_else(invalid_request_error)?;
    let state = state.inner().clone();
    let notice_state = state.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        state
            .save_encrypted_ini(expected_generation, request.plaintext, key)
            .map(|(snapshot, outcome)| SaveEncryptedIniResult {
                saved: outcome == EncryptedIniSaveOutcome::Saved,
                snapshot: snapshot.into(),
            })
            .map_err(encrypted_ini_error)
    })
    .await
    .map_err(|_| operation_error())??;
    island::publish_notice_best_effort(
        &app,
        &notice_state,
        "success",
        if result.saved {
            "Encrypted INI saved"
        } else {
            "Encrypted INI unchanged"
        },
        Vec::new(),
    );
    Ok(result)
}

#[tauri::command]
pub(crate) fn clear_encrypted_ini(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<EncryptedIniSnapshot, CommandError> {
    console::validate_window(&window)?;
    let snapshot = state
        .clear_encrypted_ini()
        .map_err(encrypted_ini_error)?
        .into();
    island::publish_notice_best_effort(
        &app,
        state.inner(),
        "success",
        "Encrypted INI editor cleared",
        Vec::new(),
    );
    Ok(snapshot)
}

fn encrypted_ini_error(error: EncryptedIniServiceError) -> CommandError {
    match error {
        EncryptedIniServiceError::Busy => CommandError::encrypted_ini(
            "encrypted_ini_busy",
            "Another encrypted INI operation is in progress.",
            Vec::new(),
        ),
        EncryptedIniServiceError::Unavailable => CommandError::encrypted_ini(
            "encrypted_ini_state_unavailable",
            "Encrypted INI editor state is unavailable.",
            Vec::new(),
        ),
        EncryptedIniServiceError::NoFile => CommandError::encrypted_ini(
            "encrypted_ini_not_open",
            "Open an INI file first",
            Vec::new(),
        ),
        EncryptedIniServiceError::StaleGeneration => CommandError::encrypted_ini(
            "encrypted_ini_stale_generation",
            "Encrypted INI editor changed; reload and try again.",
            Vec::new(),
        ),
        EncryptedIniServiceError::Document(error) => encrypted_ini_document_error(error),
    }
}

fn encrypted_ini_document_error(error: EncryptedIniError) -> CommandError {
    let (code, message_key) = match error {
        EncryptedIniError::TooLarge => (
            "encrypted_ini_too_large",
            "Encrypted INI file is too large.",
        ),
        EncryptedIniError::ReadFailed => (
            "encrypted_ini_read_failed",
            "Encrypted INI file could not be read.",
        ),
        EncryptedIniError::InvalidCiphertext => (
            "encrypted_ini_invalid_ciphertext",
            "Encrypted INI ciphertext is invalid.",
        ),
        EncryptedIniError::PlaintextTooLarge => (
            "encrypted_ini_plaintext_too_large",
            "Encrypted INI plaintext is too large.",
        ),
        EncryptedIniError::WriteFailed => (
            "encrypted_ini_write_failed",
            "Encrypted INI file could not be written.",
        ),
    };
    CommandError::encrypted_ini(code, message_key, Vec::new())
}

fn invalid_request_error() -> CommandError {
    CommandError::encrypted_ini(
        "encrypted_ini_invalid_request",
        "Encrypted INI request is invalid.",
        Vec::new(),
    )
}

fn file_dialog_error() -> CommandError {
    CommandError::encrypted_ini(
        "encrypted_ini_file_dialog_failed",
        "Encrypted INI file dialog failed.",
        Vec::new(),
    )
}

fn operation_error() -> CommandError {
    CommandError::encrypted_ini(
        "encrypted_ini_operation_failed",
        "Encrypted INI operation did not finish.",
        Vec::new(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_errors_map_to_stable_contract_codes() {
        let error = encrypted_ini_document_error(EncryptedIniError::InvalidCiphertext);
        assert_eq!(error.code, "encrypted_ini_invalid_ciphertext");
        assert_eq!(error.message_key, "Encrypted INI ciphertext is invalid.");
        assert!(error.message_arguments.is_empty());
    }

    #[test]
    fn runtime_generation_error_does_not_expose_internal_state() {
        let error = encrypted_ini_error(EncryptedIniServiceError::StaleGeneration);
        assert_eq!(error.code, "encrypted_ini_stale_generation");
        assert!(error.diagnostic_line.is_none());
    }

    #[test]
    fn unavailable_runtime_error_is_stable_and_redacted() {
        let error = encrypted_ini_error(EncryptedIniServiceError::Unavailable);

        assert_eq!(error.code, "encrypted_ini_state_unavailable");
        assert!(error.message_arguments.is_empty());
        assert!(error.diagnostic_line.is_none());
    }
}
