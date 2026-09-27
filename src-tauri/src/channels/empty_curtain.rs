use tauri::{State, WebviewWindow, ipc::Channel};

use crate::{
    channels::stream_runtime::{
        PollingStreamOutput, StreamDeliveryEndpoint, spawn_blocking_polling_stream,
        stream_registry_error, validate_subscription_id,
    },
    commands::empty_curtain::{
        empty_curtain_runtime_error, poll_plugin_changes, snapshot_with_operation,
    },
    contract::{
        CommandError, SubscriptionReceipt,
        empty_curtain::EmptyCurtainEvent,
        stream::{StreamDeliveryBody, StreamKind},
    },
    state::AppState,
    windows::console,
};

pub(crate) const EMPTY_CURTAIN_STREAM_INTERVAL_MS: u32 = 500;
#[tauri::command]
pub(crate) fn subscribe_empty_curtain(
    subscription_id: String,
    on_event: Channel<StreamDeliveryBody<EmptyCurtainEvent>>,
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
    spawn_blocking_polling_stream(
        StreamDeliveryEndpoint::new(on_event),
        state,
        registration,
        EMPTY_CURTAIN_STREAM_INTERVAL_MS,
        move |state, stop| {
            if let Err(error) = poll_plugin_changes(state, stop)
                && !stop.load(std::sync::atomic::Ordering::Acquire)
            {
                let service = state.equipment_service();
                let waiting = service.inventory.confirming().unwrap_or(false);
                if waiting {
                    if let Ok(identity) = service.inventory.identity() {
                        let _ = crate::commands::empty_curtain::settle_equipment_confirmation(
                            state, &identity, None,
                        );
                    }
                } else if !matches!(
                    error.code,
                    "plugin_session_changed" | "plugin_busy" | "equipment_snapshot_changed"
                ) {
                    let _ = service.set("error", error.message_key);
                }
            }
            let Ok((revision, operation)) = state.empty_curtain_revision_and_operation() else {
                return PollingStreamOutput::Stop;
            };
            let Ok(can_operate) = state.uses_plugin_equipment() else {
                return PollingStreamOutput::Stop;
            };
            let revision = (revision, can_operate);
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
