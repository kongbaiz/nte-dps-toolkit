use std::{sync::atomic::Ordering, thread, time::Duration};

use tauri::{State, WebviewWindow, ipc::Channel};

use crate::{
    contract::{CommandError, SubscriptionReceipt, TechnicalEvent},
    state::{AppState, TECHNICAL_STREAM_INTERVAL_MS},
    windows::hud,
};

#[tauri::command]
pub(crate) fn subscribe_technical_state(
    subscription_id: String,
    on_event: Channel<TechnicalEvent>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SubscriptionReceipt, CommandError> {
    validate_subscription_id(&subscription_id)?;
    hud::validate_window(&window)?;

    let state = state.inner().clone();
    let stop = state.begin_stream(subscription_id.clone());
    let stream_subscription_id = subscription_id.clone();
    thread::spawn(move || {
        let mut last_revision = None;
        while !stop.load(Ordering::Acquire) {
            let revision = state.stream_revision();
            if should_emit_snapshot(last_revision, revision) {
                if on_event
                    .send(TechnicalEvent::Snapshot(state.snapshot()))
                    .is_err()
                {
                    break;
                }
                last_revision = Some(revision);
            }
            thread::sleep(Duration::from_millis(u64::from(
                TECHNICAL_STREAM_INTERVAL_MS,
            )));
        }
        state.finish_stream(&stream_subscription_id, &stop);
    });

    Ok(SubscriptionReceipt {
        subscription_id,
        stream_interval_ms: TECHNICAL_STREAM_INTERVAL_MS,
    })
}

#[tauri::command]
pub(crate) fn unsubscribe_technical_state(
    subscription_id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    validate_subscription_id(&subscription_id)?;
    hud::validate_window(&window)?;
    state.stop_stream(&subscription_id);
    Ok(())
}

fn should_emit_snapshot(
    last_revision: Option<crate::state::StreamRevision>,
    current_revision: crate::state::StreamRevision,
) -> bool {
    last_revision != Some(current_revision)
}

fn validate_subscription_id(subscription_id: &str) -> Result<(), CommandError> {
    let is_valid_length = (1..=64).contains(&subscription_id.len());
    let has_valid_characters = subscription_id
        .bytes()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, b'-' | b'_'));

    if is_valid_length && has_valid_characters {
        Ok(())
    } else {
        Err(CommandError::invalid_subscription_id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_id_accepts_stable_ascii_identifier() {
        assert!(validate_subscription_id("hud_spike-01").is_ok());
    }

    #[test]
    fn subscription_id_rejects_empty_oversized_and_reserved_characters() {
        assert!(validate_subscription_id("").is_err());
        assert!(validate_subscription_id(&"a".repeat(65)).is_err());
        assert!(validate_subscription_id("hud/spike").is_err());
    }

    #[test]
    fn unchanged_revision_skips_idle_snapshot_work() {
        let state = AppState::default();
        let revision = state.stream_revision();

        assert!(should_emit_snapshot(None, revision));
        assert!(!should_emit_snapshot(Some(revision), revision));

        state.set_passthrough(true);
        assert!(should_emit_snapshot(
            Some(revision),
            state.stream_revision()
        ));
    }
}
