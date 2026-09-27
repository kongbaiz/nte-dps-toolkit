use tauri::{State, WebviewWindow, ipc::Channel};

use crate::{
    channels::stream_runtime::{
        PollingStreamOutput, StreamDeliveryEndpoint, spawn_polling_stream, stream_registry_error,
        validate_subscription_id,
    },
    commands::packets::snapshot_since,
    contract::{
        CommandError, SubscriptionReceipt,
        packets::PacketsEvent,
        stream::{StreamDeliveryBody, StreamKind},
    },
    state::AppState,
    windows::console,
};

pub(crate) const PACKETS_STREAM_INTERVAL_MS: u32 = 100;
#[tauri::command]
pub(crate) fn subscribe_packets(
    subscription_id: String,
    on_event: Channel<StreamDeliveryBody<PacketsEvent>>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SubscriptionReceipt, CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;
    state
        .packet_stream_revision()
        .map_err(CommandError::from_core)?;

    let stream_kind = StreamKind::Packets;
    let stream_key = stream_kind.stream_key(&subscription_id);
    let state = state.inner().clone();
    let registration = state
        .reserve_stream(window.label(), &stream_key)
        .map_err(stream_registry_error)?;
    let stream_generation = registration.generation();
    let mut last_revision = None;
    spawn_polling_stream(
        "nte-packets-stream",
        StreamDeliveryEndpoint::new(on_event),
        state,
        registration,
        PACKETS_STREAM_INTERVAL_MS,
        move |state| {
            let Ok(current_revision) = state.packet_stream_revision() else {
                return PollingStreamOutput::Stop;
            };
            if last_revision == Some(current_revision) {
                return PollingStreamOutput::NoChange;
            }
            let Ok((revision, replace, snapshot)) = snapshot_since(state, last_revision) else {
                return PollingStreamOutput::Stop;
            };
            let event = if replace {
                PacketsEvent::Snapshot(snapshot)
            } else {
                PacketsEvent::Append(snapshot)
            };
            last_revision = Some(revision);
            PollingStreamOutput::Event(event)
        },
    )?;

    Ok(SubscriptionReceipt::new(
        subscription_id,
        stream_kind,
        stream_generation,
        PACKETS_STREAM_INTERVAL_MS,
    ))
}
#[tauri::command]
pub(crate) fn unsubscribe_packets(
    subscription_id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;
    state
        .stop_stream(
            window.label(),
            &StreamKind::Packets.stream_key(&subscription_id),
        )
        .map_err(stream_registry_error)?;
    Ok(())
}
