use std::{sync::atomic::Ordering, thread, time::Duration};

use tauri::{State, WebviewWindow, ipc::Channel};

use crate::{
    contract::{
        CommandError, SubscriptionReceipt,
        settings::SettingsEvent,
    },
    state::AppState,
    windows::console,
};

pub(crate) const SETTINGS_STREAM_INTERVAL_MS: u32 = 200;
const STREAM_KEY_PREFIX: &str = "settings:";

#[tauri::command]
pub(crate) fn subscribe_settings(
    subscription_id: String,
    on_event: Channel<SettingsEvent>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SubscriptionReceipt, CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;

    let stream_key = stream_key(&subscription_id);
    let state = state.inner().clone();
    let stop = state.begin_stream(stream_key.clone());
    thread::spawn(move || {
        let mut last_revision = None;
        while !stop.load(Ordering::Acquire) {
            let revision = state.settings_revision();
            if should_emit_snapshot(last_revision, revision) {
                if on_event
                    .send(SettingsEvent::Snapshot(state.settings_snapshot()))
                    .is_err()
                {
                    break;
                }
                last_revision = Some(revision);
            }
            thread::sleep(Duration::from_millis(u64::from(
                SETTINGS_STREAM_INTERVAL_MS,
            )));
        }
        state.finish_stream(&stream_key, &stop);
    });

    Ok(SubscriptionReceipt {
        subscription_id,
        stream_interval_ms: SETTINGS_STREAM_INTERVAL_MS,
    })
}

#[tauri::command]
pub(crate) fn unsubscribe_settings(
    subscription_id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;
    state.stop_stream(&stream_key(&subscription_id));
    Ok(())
}

fn stream_key(subscription_id: &str) -> String {
    format!("{STREAM_KEY_PREFIX}{subscription_id}")
}

fn should_emit_snapshot(last_revision: Option<u64>, current_revision: u64) -> bool {
    last_revision != Some(current_revision)
}

fn validate_subscription_id(subscription_id: &str) -> Result<(), CommandError> {
    let valid_length = (1..=64).contains(&subscription_id.len());
    let valid_characters = subscription_id
        .bytes()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, b'-' | b'_'));
    if valid_length && valid_characters {
        Ok(())
    } else {
        Err(CommandError::invalid_subscription_id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_subscription_ids_are_bounded_ascii() {
        assert!(validate_subscription_id("settings_01").is_ok());
        assert!(validate_subscription_id("").is_err());
        assert!(validate_subscription_id("settings/01").is_err());
        assert!(validate_subscription_id(&"a".repeat(65)).is_err());
    }

    #[test]
    fn settings_stream_emits_initial_and_changed_revisions_only() {
        assert!(should_emit_snapshot(None, 0));
        assert!(!should_emit_snapshot(Some(3), 3));
        assert!(should_emit_snapshot(Some(3), 4));
    }
}
