use tauri::{State, WebviewWindow, ipc::Channel};

#[cfg(test)]
use std::{sync::atomic::Ordering, thread, time::Duration};

use crate::{
    channels::stream_runtime::{stream_registry_error, validate_subscription_id},
    contract::{
        CommandError, SubscriptionReceipt,
        history::HistoryEvent,
        stream::{StreamDeliveryBody, StreamKind},
    },
    history_runtime::{
        HISTORY_STREAM_INTERVAL, HistoryRuntime, HistoryStreamRegistration,
        history_runtime_unavailable,
    },
    state::AppState,
    windows::console,
};

pub(crate) const HISTORY_STREAM_INTERVAL_MS: u32 = HISTORY_STREAM_INTERVAL.as_millis() as u32;

#[tauri::command]
pub(crate) fn subscribe_history(
    subscription_id: String,
    on_event: Channel<StreamDeliveryBody<HistoryEvent>>,
    state: State<'_, AppState>,
    runtime: State<'_, HistoryRuntime>,
    window: WebviewWindow,
) -> Result<SubscriptionReceipt, CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;

    let stream_kind = StreamKind::History;
    let stream_key = stream_kind.stream_key(&subscription_id);
    let state = state.inner().clone();
    state
        .with_history_transaction(|| ())
        .map_err(history_runtime_unavailable)?;
    let registration = state
        .reserve_stream(window.label(), &stream_key)
        .map_err(stream_registry_error)?;
    let stream_generation = registration.generation();
    if runtime
        .register_stream(HistoryStreamRegistration {
            registration: registration.clone(),
            on_event,
        })
        .is_err()
    {
        registration.cancel();
        let _ = state.finish_stream(&registration);
        return Err(CommandError::stream_runtime_unavailable());
    }
    Ok(SubscriptionReceipt::new(
        subscription_id,
        stream_kind,
        stream_generation,
        HISTORY_STREAM_INTERVAL_MS,
    ))
}

#[cfg(test)]
pub(crate) fn run_history_stream_worker(
    state: &AppState,
    registration: &crate::state::StreamRegistration,
    mut send: impl FnMut(crate::contract::history::HistoryEvent) -> bool,
    mut wait: impl FnMut(),
) {
    let stop = registration.stop_token();
    while !registration.activation_token().load(Ordering::Acquire) && !stop.load(Ordering::Acquire)
    {
        thread::sleep(Duration::from_millis(1));
    }
    let mut last_revision = None;
    while !stop.load(Ordering::Acquire) {
        let observed_revision = state.history_revision();
        if last_revision != Some(observed_revision) {
            let (revision, snapshot) =
                match crate::history_runtime::project_history_stream_snapshot(state) {
                    Ok(projected) => projected,
                    Err(_) => break,
                };
            if stop.load(Ordering::Acquire) {
                break;
            }
            if last_revision != Some(revision) {
                let event = crate::contract::history::HistoryEvent::Snapshot(snapshot);
                if !send(event) {
                    break;
                }
                last_revision = Some(revision);
            }
        }
        wait();
    }
    let _ = state.finish_stream(registration);
}

#[tauri::command]
pub(crate) fn unsubscribe_history(
    subscription_id: String,
    state: State<'_, AppState>,
    _runtime: State<'_, HistoryRuntime>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;
    state
        .stop_stream(
            window.label(),
            &StreamKind::History.stream_key(&subscription_id),
        )
        .map_err(stream_registry_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_subscription_ids_are_bounded_ascii() {
        assert!(validate_subscription_id("history_01").is_ok());
        assert!(validate_subscription_id("").is_err());
        assert!(validate_subscription_id("history/01").is_err());
        assert!(validate_subscription_id(&"a".repeat(65)).is_err());
    }
}
