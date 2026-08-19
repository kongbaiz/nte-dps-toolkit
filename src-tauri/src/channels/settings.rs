use tauri::{State, WebviewWindow, ipc::Channel};

use crate::{
    channels::stream_runtime::{
        PollingStreamOutput, StreamDeliveryEndpoint, spawn_polling_stream, stream_registry_error,
        validate_subscription_id,
    },
    contract::{
        CommandError, SubscriptionReceipt,
        settings::SettingsEvent,
        stream::{StreamKind, StreamReadySignal},
    },
    state::{AppState, UpdateActionError},
    windows::console,
};

pub(crate) const SETTINGS_STREAM_INTERVAL_MS: u32 = 200;
#[tauri::command]
pub(crate) fn subscribe_settings(
    subscription_id: String,
    on_event: Channel<StreamReadySignal>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SubscriptionReceipt, CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;

    let stream_kind = StreamKind::Settings;
    let stream_key = stream_kind.stream_key(&subscription_id);
    let state = state.inner().clone();
    let registration = state
        .reserve_stream(window.label(), &stream_key)
        .map_err(stream_registry_error)?;
    let stream_generation = registration.generation();
    let mut last_revision = None;
    let mut last_install_blocker = None;
    spawn_polling_stream(
        "nte-settings-stream",
        StreamDeliveryEndpoint::new(stream_kind, subscription_id.clone(), on_event),
        state,
        registration,
        SETTINGS_STREAM_INTERVAL_MS,
        move |state| {
            let install_blocker = state.update_install_blocked_message_key();
            let runtime_projection_changed =
                update_runtime_projection_changed(last_install_blocker.as_ref(), &install_blocker);
            if install_blocker_changed(last_install_blocker.as_ref(), &install_blocker) {
                state.notify_update_install_blocker_changed();
            }
            last_install_blocker = Some(install_blocker);
            let revision = state.settings_revision();
            if !should_emit_snapshot(last_revision, revision) && !runtime_projection_changed {
                return PollingStreamOutput::NoChange;
            }
            last_revision = Some(revision);
            PollingStreamOutput::Event(SettingsEvent::Snapshot(state.settings_snapshot()))
        },
    )?;

    Ok(SubscriptionReceipt::new(
        subscription_id,
        stream_kind,
        stream_generation,
        SETTINGS_STREAM_INTERVAL_MS,
    ))
}

#[tauri::command]
pub(crate) fn unsubscribe_settings(
    subscription_id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;
    state
        .stop_stream(
            window.label(),
            &StreamKind::Settings.stream_key(&subscription_id),
        )
        .map_err(stream_registry_error)?;
    Ok(())
}

fn should_emit_snapshot(last_revision: Option<u64>, current_revision: u64) -> bool {
    last_revision != Some(current_revision)
}

fn install_blocker_changed(
    previous: Option<&Result<Option<&'static str>, UpdateActionError>>,
    current: &Result<Option<&'static str>, UpdateActionError>,
) -> bool {
    matches!((previous, current), (Some(Ok(previous)), Ok(current)) if previous != current)
}

fn update_runtime_projection_changed(
    previous: Option<&Result<Option<&'static str>, UpdateActionError>>,
    current: &Result<Option<&'static str>, UpdateActionError>,
) -> bool {
    previous.is_some_and(|previous| previous != current)
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

    #[test]
    fn settings_stream_advances_when_the_runtime_install_blocker_changes() {
        assert!(!install_blocker_changed(None, &Ok(None)));
        assert!(!install_blocker_changed(Some(&Ok(None)), &Ok(None)));
        assert!(install_blocker_changed(
            Some(&Ok(None)),
            &Ok(Some("capture"))
        ));
        assert!(!install_blocker_changed(
            Some(&Ok(Some("capture"))),
            &Ok(Some("capture"))
        ));
        assert!(install_blocker_changed(
            Some(&Ok(Some("capture"))),
            &Ok(None)
        ));
    }

    #[test]
    fn unavailable_update_runtime_reprojects_without_bumping_the_blocker_revision() {
        let healthy = Ok(Some("capture"));
        let unavailable = Err(UpdateActionError::RuntimeUnavailable);

        assert!(!install_blocker_changed(Some(&healthy), &unavailable));
        assert!(update_runtime_projection_changed(
            Some(&healthy),
            &unavailable
        ));
    }
}
