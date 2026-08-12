use std::{sync::atomic::Ordering, thread, time::Duration};

use tauri::{State, WebviewWindow, ipc::Channel};

use crate::{
    commands::skills::{parse_scope, snapshot},
    contract::{CommandError, SubscriptionReceipt, skills::SkillsEvent},
    state::AppState,
    windows::console,
};

pub(crate) const SKILLS_STREAM_INTERVAL_MS: u32 = 100;
const STREAM_KEY_PREFIX: &str = "skills:";

#[tauri::command]
pub(crate) fn subscribe_skills(
    subscription_id: String,
    scope: String,
    on_event: Channel<SkillsEvent>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SubscriptionReceipt, CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;
    let scope = parse_scope(&scope)?;

    let stream_key = format!("{STREAM_KEY_PREFIX}{subscription_id}");
    let state = state.inner().clone();
    let stop = state.begin_stream(window.label().to_owned(), stream_key.clone());
    thread::spawn(move || {
        let mut last_revision = None;
        while !stop.load(Ordering::Acquire) {
            let revision = state.main_dps_stream_revision();
            if last_revision != Some(revision) {
                let event = SkillsEvent::Snapshot(snapshot(&state, scope));
                if on_event.send(event).is_err() {
                    break;
                }
                last_revision = Some(revision);
            }
            thread::sleep(Duration::from_millis(u64::from(SKILLS_STREAM_INTERVAL_MS)));
        }
        state.finish_stream(&stream_key, &stop);
    });

    Ok(SubscriptionReceipt {
        subscription_id,
        stream_interval_ms: SKILLS_STREAM_INTERVAL_MS,
    })
}

#[tauri::command]
pub(crate) fn unsubscribe_skills(
    subscription_id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;
    state.stop_stream(&format!("{STREAM_KEY_PREFIX}{subscription_id}"));
    Ok(())
}

fn validate_subscription_id(subscription_id: &str) -> Result<(), CommandError> {
    let valid_length = (1..=64).contains(&subscription_id.len());
    let valid_characters = subscription_id
        .bytes()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, b'-' | b'_'));
    if valid_length && valid_characters {
        Ok(())
    } else {
        Err(CommandError::invalid_subscription_id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_subscription_ids_are_bounded_ascii() {
        assert!(validate_subscription_id("skills_01").is_ok());
        assert!(validate_subscription_id("").is_err());
        assert!(validate_subscription_id("skills/01").is_err());
        assert!(validate_subscription_id(&"a".repeat(65)).is_err());
    }
}
