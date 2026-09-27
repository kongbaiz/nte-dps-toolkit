use super::toolkit;
use crate::{contract::CommandError, state::AppState, windows::console};
use nte_dps_tool::{
    core::{
        toolkit::DataMode,
        user_characters::{self, CharacterPage, SnapshotError},
    },
    platform::toolkit::ToolkitError,
};
use serde::{Deserialize, Serialize};
use tauri::{State, WebviewWindow};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CharacterRequest {
    refresh: bool,
    expected_identity: Option<String>,
    expected_snapshot_id: Option<String>,
    offset: usize,
    selected_uid: Option<String>,
    query: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeStatus {
    process_id: u32,
    process_created_file_time: String,
    state: String,
    snapshot_id: Option<String>,
    dirty: bool,
    sdk_compatible: bool,
    #[serde(default)]
    character_refresh_supported: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CharacterResponse {
    contract_version: u32,
    connection_identity: String,
    state: String,
    dirty: bool,
    sdk_compatible: bool,
    page: Option<CharacterPage>,
}
fn failure(code: &'static str, message_key: &'static str) -> CommandError {
    CommandError {
        code,
        message_key,
        message_arguments: vec![],
        diagnostic_line: None,
    }
}
fn projection_error(e: SnapshotError) -> CommandError {
    match e {
        SnapshotError::TooLarge => failure(
            "character_snapshot_too_large",
            "The character snapshot exceeds the data limit.",
        ),
        SnapshotError::UnsupportedVersion => failure(
            "character_snapshot_version",
            "This account snapshot version is not supported.",
        ),
        SnapshotError::InvalidFormat => failure(
            "character_snapshot_invalid",
            "The character snapshot is invalid.",
        ),
        SnapshotError::SessionChanged => failure(
            "character_snapshot_changed",
            "The account snapshot changed. Reload before continuing.",
        ),
    }
}
fn validate_status(s: &NativeStatus, identity: &str) -> Result<(), CommandError> {
    if !matches!(
        s.state.as_str(),
        "idle" | "queued" | "reading" | "completed" | "failed" | "canceled" | "stopped"
    ) || s
        .snapshot_id
        .as_ref()
        .is_some_and(|s| s.is_empty() || s.len() > 128)
    {
        return Err(projection_error(SnapshotError::InvalidFormat));
    }
    let created = user_characters::decimal(&serde_json::Value::String(
        s.process_created_file_time.clone(),
    ))
    .map_err(projection_error)?;
    if format!("{}:{}", s.process_id, created) != identity {
        return Err(projection_error(SnapshotError::SessionChanged));
    }
    if s.state == "completed" && !s.dirty && s.snapshot_id.is_none() {
        return Err(projection_error(SnapshotError::InvalidFormat));
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn get_user_characters(
    request: CharacterRequest,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<CharacterResponse, CommandError> {
    console::validate_window(&window)?;
    if request.offset > 2048
        || request.query.len() > 128
        || request.selected_uid.as_ref().is_some_and(|s| s.len() > 32)
        || request
            .expected_identity
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 64)
        || request
            .expected_snapshot_id
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 128)
        || (request.refresh && request.expected_identity.is_none())
    {
        return Err(projection_error(SnapshotError::InvalidFormat));
    }
    if state.data_mode() != DataMode::Plugin {
        return Err(failure(
            "character_plugin_mode_required",
            "Select Plugin mode on the home page to read character snapshots.",
        ));
    }
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Reuse the capacity-one control reservation; no hot state lock spans IPC.
        let _permit = state
            .reserve_plugin_control()
            .map_err(CommandError::from_core)?;
        if state.data_mode() != DataMode::Plugin {
            return Err(toolkit::unsupported());
        }
        let (client, capabilities) = toolkit::client()?;
        let identity = client.identity();
        if request
            .expected_identity
            .as_ref()
            .is_some_and(|s| s != &identity)
        {
            return Err(projection_error(SnapshotError::SessionChanged));
        }
        if ![302, 303].iter().all(|c| capabilities.contains(c))
            || (request.refresh && !capabilities.contains(&301))
        {
            return Err(toolkit::unsupported());
        }
        let prior: NativeStatus = client.json(302, 0, "", &|| false).map_err(toolkit::error)?;
        validate_status(&prior, &identity)?;
        let status: NativeStatus = if request.refresh {
            client
                .json(
                    301,
                    u32::from(prior.character_refresh_supported),
                    "",
                    &|| false,
                )
                .map_err(toolkit::error)?
        } else {
            prior
        };
        validate_status(&status, &identity)?;
        let page = if status.state == "completed" && !status.dirty && status.sdk_compatible {
            let id = status
                .snapshot_id
                .as_deref()
                .ok_or_else(|| projection_error(SnapshotError::InvalidFormat))?;
            if request
                .expected_snapshot_id
                .as_ref()
                .is_some_and(|s| s != id)
            {
                return Err(projection_error(SnapshotError::SessionChanged));
            }
            // Read the complete paged Toolkit blob, NOT release_action's 8 KiB preview.
            // Pass native snapshot identity to reject concurrent refresh/invalidation.
            let bytes = client.call(303, 0, id, &|| false).map_err(toolkit::error)?;
            let page = user_characters::project(
                &bytes,
                &identity,
                id,
                request.offset,
                request.selected_uid.as_deref(),
                &request.query,
            )
            .map_err(projection_error)?;
            let after: NativeStatus = client.json(302, 0, "", &|| false).map_err(toolkit::error)?;
            validate_status(&after, &identity)?;
            if after.dirty || after.state != "completed" || after.snapshot_id != status.snapshot_id
            {
                return Err(projection_error(SnapshotError::SessionChanged));
            }
            Some(page)
        } else {
            None
        };
        if state.data_mode() != DataMode::Plugin {
            return Err(toolkit::unsupported());
        }
        Ok(CharacterResponse {
            contract_version: user_characters::USER_CHARACTERS_CONTRACT_VERSION,
            connection_identity: identity,
            state: status.state,
            dirty: status.dirty,
            sdk_compatible: status.sdk_compatible,
            page,
        })
    })
    .await
    .map_err(|_| toolkit::error(ToolkitError::Failed))?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_rejects_restart_and_missing_published_identity() {
        let mut s = NativeStatus {
            process_id: 42,
            process_created_file_time: "99".into(),
            state: "completed".into(),
            snapshot_id: Some("provider:1".into()),
            dirty: false,
            sdk_compatible: true,
            character_refresh_supported: false,
        };
        assert!(validate_status(&s, "42:99").is_ok());
        assert!(validate_status(&s, "42:100").is_err());
        s.snapshot_id = None;
        assert!(validate_status(&s, "42:99").is_err());
        s.state = "PRIVATE_PATH".into();
        assert!(validate_status(&s, "42:99").is_err());
    }
}
