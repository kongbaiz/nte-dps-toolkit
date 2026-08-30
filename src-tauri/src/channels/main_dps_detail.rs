use tauri::{State, WebviewWindow, ipc::Channel};

use crate::{
    channels::stream_runtime::{
        PollingStreamOutput, StreamDeliveryEndpoint, spawn_polling_stream, stream_registry_error,
        validate_subscription_id,
    },
    contract::{
        CommandError, SubscriptionReceipt,
        main_dps_detail::{MAIN_DPS_DETAIL_PAGE_LIMIT, MainDpsDetailSnapshot},
        stream::{StreamDeliveryBody, StreamKind},
    },
    state::AppState,
    windows::combat_details,
};

pub(crate) const MAIN_DPS_DETAIL_STREAM_INTERVAL_MS: u32 = 250;
#[tauri::command]
pub(crate) fn subscribe_main_dps_detail(
    subscription_id: String,
    on_event: Channel<StreamDeliveryBody<MainDpsDetailSnapshot>>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SubscriptionReceipt, CommandError> {
    validate_subscription_id(&subscription_id)?;
    combat_details::validate_window(&window)?;
    let kind = combat_details::window_kind(&window)?;
    state
        .main_dps_stream_revision()
        .map_err(CommandError::from_core)?;
    let stream_kind = StreamKind::MainDpsDetail;
    let stream_key = stream_kind.stream_key(&subscription_id);
    let state = state.inner().clone();
    let registration = state
        .reserve_stream(window.label(), &stream_key)
        .map_err(stream_registry_error)?;
    let stream_generation = registration.generation();
    let mut last_revision = None;
    spawn_polling_stream(
        "nte-main-dps-detail-stream",
        StreamDeliveryEndpoint::new(on_event),
        state,
        registration,
        MAIN_DPS_DETAIL_STREAM_INTERVAL_MS,
        move |state| {
            let Ok(revision) = state.main_dps_stream_revision() else {
                return PollingStreamOutput::Stop;
            };
            let visible = match window.is_visible() {
                Ok(visible) => visible,
                Err(_) => return PollingStreamOutput::Stop,
            };
            if !visible || last_revision == Some(revision) {
                return PollingStreamOutput::NoChange;
            }
            let Ok(next) =
                MainDpsDetailSnapshot::from_state(state, kind, 0, MAIN_DPS_DETAIL_PAGE_LIMIT)
            else {
                return PollingStreamOutput::Stop;
            };
            last_revision = Some(revision);
            PollingStreamOutput::Event(next)
        },
    )?;
    Ok(SubscriptionReceipt::new(
        subscription_id,
        stream_kind,
        stream_generation,
        MAIN_DPS_DETAIL_STREAM_INTERVAL_MS,
    ))
}
#[tauri::command]
pub(crate) fn unsubscribe_main_dps_detail(
    subscription_id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    validate_subscription_id(&subscription_id)?;
    combat_details::validate_window(&window)?;
    state
        .stop_stream(
            window.label(),
            &StreamKind::MainDpsDetail.stream_key(&subscription_id),
        )
        .map_err(stream_registry_error)?;
    Ok(())
}
