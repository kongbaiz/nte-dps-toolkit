use tauri::{State, WebviewWindow, ipc::Channel};

use crate::{
    channels::stream_runtime::validate_subscription_id,
    contract::{CommandError, SubscriptionReceipt, history::HistoryEvent, stream::StreamKind},
    history_runtime::{HISTORY_STREAM_INTERVAL, HistoryRuntime, HistoryStreamRegistration},
    state::AppState,
    windows::console,
};

pub(crate) const HISTORY_STREAM_INTERVAL_MS: u32 = HISTORY_STREAM_INTERVAL.as_millis() as u32;

#[tauri::command]
pub(crate) fn subscribe_history(
    subscription_id: String,
    on_event: Channel<HistoryEvent>,
    state: State<'_, AppState>,
    runtime: State<'_, HistoryRuntime>,
    window: WebviewWindow,
) -> Result<SubscriptionReceipt, CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;

    let stream_kind = StreamKind::History;
    let stream_key = stream_kind.stream_key(&subscription_id);
    let state = state.inner().clone();
    state.with_history_transaction(|| ())?;
    let registration = state.reserve_stream(window.label(), &stream_key)?;
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
#[tauri::command]
pub(crate) fn unsubscribe_history(
    subscription_id: String,
    state: State<'_, AppState>,
    _runtime: State<'_, HistoryRuntime>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;
    state.stop_stream(
        window.label(),
        &StreamKind::History.stream_key(&subscription_id),
    )?;
    Ok(())
}
