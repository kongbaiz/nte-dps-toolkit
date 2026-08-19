use tauri::{State, WebviewWindow, ipc::Channel};

use crate::{
    channels::stream_runtime::{
        PollingStreamOutput, StreamDeliveryEndpoint, spawn_polling_stream, stream_registry_error,
        validate_subscription_id,
    },
    commands::empty_curtain::{empty_curtain_runtime_error, snapshot_with_operation},
    contract::{
        CommandError, SubscriptionReceipt,
        empty_curtain::EmptyCurtainEvent,
        stream::{StreamKind, StreamReadySignal},
    },
    state::AppState,
    windows::console,
};

pub(crate) const EMPTY_CURTAIN_STREAM_INTERVAL_MS: u32 = 100;
#[tauri::command]
pub(crate) fn subscribe_empty_curtain(
    subscription_id: String,
    on_event: Channel<StreamReadySignal>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SubscriptionReceipt, CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;
    state
        .empty_curtain_revision_and_operation()
        .map_err(empty_curtain_runtime_error)?;
    let stream_kind = StreamKind::EmptyCurtain;
    let stream_key = stream_kind.stream_key(&subscription_id);
    let state = state.inner().clone();
    let registration = state
        .reserve_stream(window.label(), &stream_key)
        .map_err(stream_registry_error)?;
    let stream_generation = registration.generation();
    let mut last_revision = None;
    spawn_polling_stream(
        "nte-empty-curtain-stream",
        StreamDeliveryEndpoint::new(stream_kind, subscription_id.clone(), on_event),
        state,
        registration,
        EMPTY_CURTAIN_STREAM_INTERVAL_MS,
        move |state| {
            let Ok((revision, operation)) = state.empty_curtain_revision_and_operation() else {
                return PollingStreamOutput::Stop;
            };
            if last_revision == Some(revision) {
                return PollingStreamOutput::NoChange;
            }
            let Ok(next) = snapshot_with_operation(state, operation) else {
                return PollingStreamOutput::Stop;
            };
            last_revision = Some(revision);
            PollingStreamOutput::Event(EmptyCurtainEvent::Snapshot(next))
        },
    )?;
    Ok(SubscriptionReceipt::new(
        subscription_id,
        stream_kind,
        stream_generation,
        EMPTY_CURTAIN_STREAM_INTERVAL_MS,
    ))
}

#[tauri::command]
pub(crate) fn unsubscribe_empty_curtain(
    subscription_id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;
    state
        .stop_stream(
            window.label(),
            &StreamKind::EmptyCurtain.stream_key(&subscription_id),
        )
        .map_err(stream_registry_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_ids_are_bounded_ascii() {
        assert!(validate_subscription_id("empty_curtain_01").is_ok());
        assert!(validate_subscription_id("").is_err());
        assert!(validate_subscription_id("empty/curtain").is_err());
        assert!(validate_subscription_id(&"a".repeat(65)).is_err());
    }
}
