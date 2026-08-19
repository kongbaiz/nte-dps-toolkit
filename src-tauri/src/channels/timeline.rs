use tauri::{State, WebviewWindow, ipc::Channel};

use crate::{
    channels::stream_runtime::{
        PollingStreamOutput, StreamDeliveryEndpoint, spawn_polling_stream, stream_registry_error,
        validate_subscription_id,
    },
    commands::timeline::{parse_scope, snapshot},
    contract::{
        CommandError, SubscriptionReceipt,
        stream::{StreamKind, StreamReadySignal},
        timeline::TimelineEvent,
    },
    state::AppState,
    windows::console,
};

pub(crate) const TIMELINE_STREAM_INTERVAL_MS: u32 = 100;
#[tauri::command]
pub(crate) fn subscribe_timeline(
    subscription_id: String,
    scope: String,
    on_event: Channel<StreamReadySignal>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SubscriptionReceipt, CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;
    let scope = parse_scope(&scope)?;
    state
        .main_dps_stream_revision()
        .map_err(CommandError::from_core)?;

    let stream_kind = StreamKind::Timeline;
    let stream_key = stream_kind.stream_key(&subscription_id);
    let state = state.inner().clone();
    let registration = state
        .reserve_stream(window.label(), &stream_key)
        .map_err(stream_registry_error)?;
    let stream_generation = registration.generation();
    let mut last_revision = None;
    spawn_polling_stream(
        "nte-timeline-stream",
        StreamDeliveryEndpoint::new(stream_kind, subscription_id.clone(), on_event),
        state,
        registration,
        TIMELINE_STREAM_INTERVAL_MS,
        move |state| {
            let Ok(revision) = state.main_dps_stream_revision() else {
                return PollingStreamOutput::Stop;
            };
            if last_revision == Some(revision) {
                return PollingStreamOutput::NoChange;
            }
            let Ok(next) = snapshot(state, scope) else {
                return PollingStreamOutput::Stop;
            };
            last_revision = Some(revision);
            PollingStreamOutput::Event(TimelineEvent::Snapshot(next))
        },
    )?;

    Ok(SubscriptionReceipt::new(
        subscription_id,
        stream_kind,
        stream_generation,
        TIMELINE_STREAM_INTERVAL_MS,
    ))
}

#[tauri::command]
pub(crate) fn unsubscribe_timeline(
    subscription_id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;
    state
        .stop_stream(
            window.label(),
            &StreamKind::Timeline.stream_key(&subscription_id),
        )
        .map_err(stream_registry_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_subscription_ids_are_bounded_ascii() {
        assert!(validate_subscription_id("timeline_01").is_ok());
        assert!(validate_subscription_id("").is_err());
        assert!(validate_subscription_id("timeline/01").is_err());
        assert!(validate_subscription_id(&"a".repeat(65)).is_err());
    }
}
