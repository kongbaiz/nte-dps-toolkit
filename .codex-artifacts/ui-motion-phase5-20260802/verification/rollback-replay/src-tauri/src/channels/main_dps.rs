use std::{sync::atomic::Ordering, thread, time::Duration};

use tauri::{State, WebviewWindow, ipc::Channel};

use crate::{
    commands::main_dps::snapshot,
    contract::{CommandError, SubscriptionReceipt, main_dps::MainDpsEvent},
    state::AppState,
    windows::main_dps,
};

pub(crate) const MAIN_DPS_STREAM_INTERVAL_MS: u32 = 100;
const STREAM_KEY_PREFIX: &str = "main-dps:";

#[tauri::command]
pub(crate) fn subscribe_main_dps(
    subscription_id: String,
    on_event: Channel<MainDpsEvent>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SubscriptionReceipt, CommandError> {
    validate_subscription_id(&subscription_id)?;
    main_dps::validate_window(&window)?;
    let stream_key = format!("{STREAM_KEY_PREFIX}{subscription_id}");
    let state = state.inner().clone();
    let stop = state.begin_stream(stream_key.clone());
    thread::spawn(move || {
        let mut last_revision = None;
        let mut last_game_detected = None;
        let mut last_empty = true;
        let mut empty_probe_ticks = 0_u8;
        while !stop.load(Ordering::Acquire) {
            let revision = state.main_dps_stream_revision();
            if last_revision != Some(revision) {
                let next = snapshot(&state);
                last_game_detected = Some(next.game_detected);
                last_empty = next.readout.data_state == "empty";
                empty_probe_ticks = 0;
                if on_event.send(MainDpsEvent::Snapshot(next)).is_err() {
                    break;
                }
                last_revision = Some(revision);
            } else {
                empty_probe_ticks = empty_probe_ticks.saturating_add(1);
                if last_empty && empty_probe_ticks >= 10 {
                    empty_probe_ticks = 0;
                    let next = snapshot(&state);
                    if next.readout.data_state == "empty"
                        && last_game_detected != Some(next.game_detected)
                    {
                        last_game_detected = Some(next.game_detected);
                        last_empty = true;
                        if on_event.send(MainDpsEvent::Snapshot(next)).is_err() {
                            break;
                        }
                    }
                }
            }
            thread::sleep(Duration::from_millis(u64::from(
                MAIN_DPS_STREAM_INTERVAL_MS,
            )));
        }
        state.finish_stream(&stream_key, &stop);
    });
    Ok(SubscriptionReceipt {
        subscription_id,
        stream_interval_ms: MAIN_DPS_STREAM_INTERVAL_MS,
    })
}

#[tauri::command]
pub(crate) fn unsubscribe_main_dps(
    subscription_id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    validate_subscription_id(&subscription_id)?;
    main_dps::validate_window(&window)?;
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
    fn main_dps_subscription_ids_are_bounded_ascii() {
        assert!(validate_subscription_id("main_dps_01").is_ok());
        assert!(validate_subscription_id("").is_err());
        assert!(validate_subscription_id("main/dps").is_err());
        assert!(validate_subscription_id(&"a".repeat(65)).is_err());
    }

}
