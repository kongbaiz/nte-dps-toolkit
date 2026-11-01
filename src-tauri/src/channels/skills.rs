use nte_dps_tool::core::timeline::TimelineScope;
use tauri::{State, WebviewWindow, ipc::Channel};

use crate::{
    channels::stream_runtime::{
        PollingStreamOutput, StreamDeliveryEndpoint, spawn_polling_stream, validate_subscription_id,
    },
    commands::skills::snapshot,
    contract::{CommandError, SubscriptionReceipt, skills::SkillsEvent, stream::StreamKind},
    state::AppState,
    windows::console,
};

pub(crate) const SKILLS_STREAM_INTERVAL_MS: u32 = 100;
#[tauri::command]
pub(crate) fn subscribe_skills(
    subscription_id: String,
    scope: TimelineScope,
    on_event: Channel<SkillsEvent>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SubscriptionReceipt, CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;
    state.main_dps_stream_revision()?;

    let stream_kind = StreamKind::Skills;
    let stream_key = stream_kind.stream_key(&subscription_id);
    let state = state.inner().clone();
    let registration = state.reserve_stream(window.label(), &stream_key)?;
    let stream_generation = registration.generation();
    let mut last_revision = None;
    spawn_polling_stream(
        "nte-skills-stream",
        StreamDeliveryEndpoint::new(on_event),
        state,
        registration,
        SKILLS_STREAM_INTERVAL_MS,
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
            PollingStreamOutput::Event(SkillsEvent::Snapshot(next))
        },
    )?;

    Ok(SubscriptionReceipt::new(
        subscription_id,
        stream_kind,
        stream_generation,
        SKILLS_STREAM_INTERVAL_MS,
    ))
}
#[tauri::command]
pub(crate) fn unsubscribe_skills(
    subscription_id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;
    state.stop_stream(
        window.label(),
        &StreamKind::Skills.stream_key(&subscription_id),
    )?;
    Ok(())
}
