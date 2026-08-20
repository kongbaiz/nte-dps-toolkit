use tauri::{State, WebviewWindow, ipc::Channel};

use crate::{
    channels::stream_runtime::{
        PollingStreamOutput, StreamDeliveryEndpoint, spawn_polling_stream, stream_registry_error,
        validate_subscription_id,
    },
    contract::{
        CommandError, SubscriptionReceipt, TechnicalEvent,
        stream::{StreamDeliveryBody, StreamKind},
    },
    state::{AppState, TECHNICAL_STREAM_INTERVAL_MS},
    windows::hud,
};

#[tauri::command]
pub(crate) fn subscribe_technical_state(
    subscription_id: String,
    on_event: Channel<StreamDeliveryBody<TechnicalEvent>>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SubscriptionReceipt, CommandError> {
    validate_subscription_id(&subscription_id)?;
    hud::validate_window(&window)?;
    state.snapshot().map_err(CommandError::from_core)?;

    let stream_kind = StreamKind::Technical;
    let stream_key = stream_kind.stream_key(&subscription_id);
    let state = state.inner().clone();
    let registration = state
        .reserve_stream(window.label(), &stream_key)
        .map_err(stream_registry_error)?;
    let stream_generation = registration.generation();
    let mut last_revision = None;
    spawn_polling_stream(
        "nte-technical-stream",
        StreamDeliveryEndpoint::new(on_event),
        state,
        registration,
        TECHNICAL_STREAM_INTERVAL_MS,
        move |state| {
            let revision = state.stream_revision();
            if !should_emit_snapshot(last_revision, revision) {
                return PollingStreamOutput::NoChange;
            }
            let Ok(next) = state.snapshot() else {
                return PollingStreamOutput::Stop;
            };
            last_revision = Some(revision);
            PollingStreamOutput::Event(TechnicalEvent::Snapshot(next))
        },
    )?;

    Ok(SubscriptionReceipt::new(
        subscription_id,
        stream_kind,
        stream_generation,
        TECHNICAL_STREAM_INTERVAL_MS,
    ))
}

#[tauri::command]
pub(crate) fn unsubscribe_technical_state(
    subscription_id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    validate_subscription_id(&subscription_id)?;
    hud::validate_window(&window)?;
    state
        .stop_stream(
            window.label(),
            &StreamKind::Technical.stream_key(&subscription_id),
        )
        .map_err(stream_registry_error)?;
    Ok(())
}

fn should_emit_snapshot(
    last_revision: Option<crate::state::StreamRevision>,
    current_revision: crate::state::StreamRevision,
) -> bool {
    last_revision != Some(current_revision)
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
