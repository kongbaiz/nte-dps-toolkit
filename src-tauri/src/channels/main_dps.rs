use std::{sync::atomic::Ordering, thread, time::Duration};

use nte_dps_tool::core::live_capture::LiveCapturePhase;
use tauri::{AppHandle, State, WebviewWindow, ipc::Channel};

use crate::{
    commands::main_dps::snapshot,
    contract::{CommandError, SubscriptionReceipt, main_dps::MainDpsEvent},
    state::AppState,
    windows::{island, main_dps},
};

pub(crate) const MAIN_DPS_STREAM_INTERVAL_MS: u32 = 100;
const STREAM_KEY_PREFIX: &str = "main-dps:";

#[tauri::command]
pub(crate) fn subscribe_main_dps(
    subscription_id: String,
    on_event: Channel<MainDpsEvent>,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SubscriptionReceipt, CommandError> {
    validate_subscription_id(&subscription_id)?;
    main_dps::validate_window(&window)?;
    let stream_key = format!("{STREAM_KEY_PREFIX}{subscription_id}");
    let state = state.inner().clone();
    let stop = state.begin_stream(window.label().to_owned(), stream_key.clone());
    thread::spawn(move || {
        let mut last_revision = None;
        let mut last_game_detected = None;
        let mut last_game_detection_status = None;
        let mut last_empty = true;
        let mut empty_probe_ticks = 0_u8;
        let mut last_capture_lifecycle = None;
        while !stop.load(Ordering::Acquire) {
            let revision = state.main_dps_stream_revision();
            if last_revision != Some(revision) {
                let next = snapshot(&state);
                last_game_detected = Some(next.game_detected);
                last_game_detection_status = Some(next.game_detection_status);
                last_empty = next.readout.data_state == "empty";
                empty_probe_ticks = 0;
                let lifecycle = (state.capture_phase(), state.replay_running());
                if let Some(previous) = last_capture_lifecycle {
                    publish_capture_transition(&app, &state, previous, lifecycle);
                }
                last_capture_lifecycle = Some(lifecycle);
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
                        && (last_game_detected != Some(next.game_detected)
                            || last_game_detection_status != Some(next.game_detection_status))
                    {
                        last_game_detected = Some(next.game_detected);
                        last_game_detection_status = Some(next.game_detection_status);
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

fn publish_capture_transition(
    app: &AppHandle,
    state: &AppState,
    previous: (LiveCapturePhase, bool),
    current: (LiveCapturePhase, bool),
) {
    let notice = capture_transition_notice(previous, current);
    if let Some((tone, message_key)) = notice {
        island::publish_notice_best_effort(app, state, tone, message_key, Vec::new());
    }
}

fn capture_transition_notice(
    previous: (LiveCapturePhase, bool),
    current: (LiveCapturePhase, bool),
) -> Option<(&'static str, &'static str)> {
    if previous.1 && !current.1 && current.0 == LiveCapturePhase::Stopped {
        return Some((
            "success",
            "Import complete; see parse quality on the diagnostics page",
        ));
    }
    if current.0 == LiveCapturePhase::Failed && previous.0 != LiveCapturePhase::Failed {
        return Some(("error", "Capture failed"));
    }
    if previous.0 == LiveCapturePhase::Starting && current.0 == LiveCapturePhase::Running {
        return Some(("success", "Live capture started, BPF is being determined"));
    }
    if matches!(
        previous.0,
        LiveCapturePhase::Running | LiveCapturePhase::Stopping
    ) && current.0 == LiveCapturePhase::Stopped
        && !previous.1
    {
        return Some(("success", "Capture stopped"));
    }
    None
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

    #[test]
    fn capture_lifecycle_notices_cover_replay_and_live_completion() {
        assert_eq!(
            capture_transition_notice(
                (LiveCapturePhase::Running, true),
                (LiveCapturePhase::Stopped, false),
            ),
            Some((
                "success",
                "Import complete; see parse quality on the diagnostics page",
            ))
        );
        assert_eq!(
            capture_transition_notice(
                (LiveCapturePhase::Stopping, false),
                (LiveCapturePhase::Stopped, false),
            ),
            Some(("success", "Capture stopped"))
        );
    }
}
