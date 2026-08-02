use std::{sync::atomic::Ordering, thread, time::Duration};

use tauri::{State, WebviewWindow, ipc::Channel};

use crate::{
    contract::{CommandError, SubscriptionReceipt, main_dps_detail::MainDpsDetailSnapshot},
    state::AppState,
    windows::combat_details,
};

pub(crate) const MAIN_DPS_DETAIL_STREAM_INTERVAL_MS: u32 = 250;
const STREAM_KEY_PREFIX: &str = "main-dps-detail:";
const STREAM_PAGE_LIMIT: usize = 250;

#[tauri::command]
pub(crate) fn subscribe_main_dps_detail(
    subscription_id: String,
    on_event: Channel<MainDpsDetailSnapshot>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SubscriptionReceipt, CommandError> {
    validate_subscription_id(&subscription_id)?;
    combat_details::validate_window(&window)?;
    let kind = combat_details::window_kind(&window)?;
    let stream_key = format!("{STREAM_KEY_PREFIX}{subscription_id}");
    let state = state.inner().clone();
    let stop = state.begin_stream(stream_key.clone());
    thread::spawn(move || {
        let mut last_revision = None;
        while !stop.load(Ordering::Acquire) {
            let revision = state.main_dps_stream_revision();
            let visible = window.is_visible().unwrap_or(false);
            if visible && last_revision != Some(revision) {
                let next = MainDpsDetailSnapshot::from_state(&state, kind, 0, STREAM_PAGE_LIMIT);
                if on_event.send(next).is_err() {
                    break;
                }
                last_revision = Some(revision);
            }
            thread::sleep(Duration::from_millis(u64::from(
                MAIN_DPS_DETAIL_STREAM_INTERVAL_MS,
            )));
        }
        state.finish_stream(&stream_key, &stop);
    });
    Ok(SubscriptionReceipt {
        subscription_id,
        stream_interval_ms: MAIN_DPS_DETAIL_STREAM_INTERVAL_MS,
    })
}

#[tauri::command]
pub(crate) fn unsubscribe_main_dps_detail(
    subscription_id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    validate_subscription_id(&subscription_id)?;
    combat_details::validate_window(&window)?;
    state.stop_stream(&format!("{STREAM_KEY_PREFIX}{subscription_id}"));
    Ok(())
}

fn validate_subscription_id(value: &str) -> Result<(), CommandError> {
    if (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        Ok(())
    } else {
        Err(CommandError::invalid_subscription_id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_subscription_ids_are_bounded_ascii() {
        assert!(validate_subscription_id("detail_01").is_ok());
        assert!(validate_subscription_id("").is_err());
        assert!(validate_subscription_id("detail/01").is_err());
        assert!(validate_subscription_id(&"a".repeat(65)).is_err());
    }
}
