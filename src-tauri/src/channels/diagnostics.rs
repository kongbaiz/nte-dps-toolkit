use tauri::{State, WebviewWindow, ipc::Channel};

use crate::{
    channels::stream_runtime::{
        PollingStreamOutput, StreamDeliveryEndpoint, spawn_polling_stream, stream_registry_error,
        validate_subscription_id,
    },
    commands::diagnostics::snapshot,
    contract::{
        CommandError, SubscriptionReceipt,
        diagnostics::DiagnosticsEvent,
        stream::{StreamKind, StreamReadySignal},
    },
    state::AppState,
    windows::console,
};

pub(crate) const DIAGNOSTICS_STREAM_INTERVAL_MS: u32 = 500;
#[tauri::command]
pub(crate) fn subscribe_diagnostics(
    subscription_id: String,
    on_event: Channel<StreamReadySignal>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SubscriptionReceipt, CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;
    state
        .diagnostics_revision()
        .map_err(CommandError::from_core)?;

    let stream_kind = StreamKind::Diagnostics;
    let stream_key = stream_kind.stream_key(&subscription_id);
    let state = state.inner().clone();
    let registration = state
        .reserve_stream(window.label(), &stream_key)
        .map_err(stream_registry_error)?;
    let stream_generation = registration.generation();
    let mut last_revision = None;
    spawn_polling_stream(
        "nte-diagnostics-stream",
        StreamDeliveryEndpoint::new(stream_kind, subscription_id.clone(), on_event),
        state,
        registration,
        DIAGNOSTICS_STREAM_INTERVAL_MS,
        move |state| {
            let Ok(revision) = state.diagnostics_revision() else {
                return PollingStreamOutput::Stop;
            };
            if last_revision == Some(revision) {
                return PollingStreamOutput::NoChange;
            }
            let Ok(next) = snapshot(state) else {
                return PollingStreamOutput::Stop;
            };
            last_revision = Some(revision);
            PollingStreamOutput::Event(DiagnosticsEvent::Snapshot(next))
        },
    )?;

    Ok(SubscriptionReceipt::new(
        subscription_id,
        stream_kind,
        stream_generation,
        DIAGNOSTICS_STREAM_INTERVAL_MS,
    ))
}

#[tauri::command]
pub(crate) fn unsubscribe_diagnostics(
    subscription_id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;
    state
        .stop_stream(
            window.label(),
            &StreamKind::Diagnostics.stream_key(&subscription_id),
        )
        .map_err(stream_registry_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_subscription_ids_are_bounded_ascii() {
        assert!(validate_subscription_id("diagnostics_01").is_ok());
        assert!(validate_subscription_id("").is_err());
        assert!(validate_subscription_id("diagnostics/01").is_err());
        assert!(validate_subscription_id(&"a".repeat(65)).is_err());
    }
}
