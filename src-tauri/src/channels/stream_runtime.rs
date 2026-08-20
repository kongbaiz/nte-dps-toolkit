use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::atomic::Ordering,
    time::Duration,
};

use serde::Serialize;
use tauri::ipc::Channel;
use tokio::time::{MissedTickBehavior, interval};

use crate::{
    contract::{
        CommandError,
        stream::{MAX_EVENTS_PER_STREAM_DELIVERY, MAX_STREAM_DELIVERY_BYTES, StreamDeliveryBody},
    },
    state::{AppState, StreamRegistration, StreamRegistryError},
};

pub(crate) fn is_valid_subscription_id(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub(crate) fn validate_subscription_id(value: &str) -> Result<(), CommandError> {
    if is_valid_subscription_id(value) {
        Ok(())
    } else {
        Err(CommandError::invalid_subscription_id())
    }
}

pub(crate) fn stream_registry_error(_: StreamRegistryError) -> CommandError {
    CommandError::stream_runtime_unavailable()
}

/// Result of one revision-aware projection pass.
pub(crate) enum PollingStreamOutput<T> {
    NoChange,
    Event(T),
    Events(Vec<T>),
    Stop,
}

pub(crate) struct StreamDeliveryEndpoint<T> {
    channel: Channel<StreamDeliveryBody<T>>,
}

impl<T> StreamDeliveryEndpoint<T> {
    pub(crate) fn new(channel: Channel<StreamDeliveryBody<T>>) -> Self {
        Self { channel }
    }
}

/// Starts one lightweight async owner per subscription.
///
/// The registry owns cancellation and replacement. Tokio's skipped-tick policy
/// coalesces slow projections, while the Tauri Channel preserves event ordering.
/// A disconnected Channel or invalid projection ends the subscription and
/// removes its registry entry.
pub(crate) fn spawn_polling_stream<T, Poll>(
    stream_name: &'static str,
    delivery: StreamDeliveryEndpoint<T>,
    state: AppState,
    registration: StreamRegistration,
    interval_ms: u32,
    mut poll: Poll,
) -> Result<(), CommandError>
where
    T: Serialize + Send + 'static,
    Poll: FnMut(&AppState) -> PollingStreamOutput<T> + Send + 'static,
{
    match state.activate_stream(&registration) {
        Ok(true) => {}
        Ok(false) | Err(_) => {
            registration.cancel();
            let _ = state.finish_stream(&registration);
            return Err(CommandError::stream_runtime_unavailable());
        }
    }

    let task_state = state.clone();
    let task_registration = registration.clone();
    tauri::async_runtime::spawn(async move {
        let stop = task_registration.stop_token();
        let active = task_registration.activation_token();
        let mut ticker = interval(Duration::from_millis(u64::from(interval_ms.max(1))));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        while !stop.load(Ordering::Acquire) {
            ticker.tick().await;
            if stop.load(Ordering::Acquire) || !active.load(Ordering::Acquire) {
                continue;
            }

            let output = catch_unwind(AssertUnwindSafe(|| poll(&task_state)));
            let events = match output {
                Ok(PollingStreamOutput::NoChange) => continue,
                Ok(PollingStreamOutput::Event(event)) => vec![event],
                Ok(PollingStreamOutput::Events(events)) => events,
                Ok(PollingStreamOutput::Stop) => break,
                Err(_) => {
                    log::error!("{stream_name} stream projection panicked; subscription stopped");
                    break;
                }
            };

            if stop.load(Ordering::Acquire) {
                break;
            }
            if events.is_empty() {
                continue;
            }
            if events.len() > MAX_EVENTS_PER_STREAM_DELIVERY {
                log::error!(
                    "{stream_name} stream produced {} events; maximum is {}",
                    events.len(),
                    MAX_EVENTS_PER_STREAM_DELIVERY
                );
                break;
            }

            let body = StreamDeliveryBody::new(events);
            if !delivery_fits(&body) {
                log::error!(
                    "{stream_name} stream delivery exceeded the {} byte limit",
                    MAX_STREAM_DELIVERY_BYTES
                );
                break;
            }
            if delivery.channel.send(body).is_err() {
                break;
            }
        }

        task_registration.cancel();
        if task_state.finish_stream(&task_registration).is_err() {
            log::warn!("Stream registry cleanup reset after an interrupted update");
        }
    });

    Ok(())
}

fn delivery_fits<T: Serialize>(delivery: &StreamDeliveryBody<T>) -> bool {
    serde_json::to_vec(delivery)
        .map(|encoded| encoded.len() <= MAX_STREAM_DELIVERY_BYTES)
        .unwrap_or(false)
}

#[cfg(test)]
pub(crate) fn serialize_stream_events<T: Serialize>(events: Vec<T>) -> std::io::Result<Vec<u8>> {
    let bytes =
        serde_json::to_vec(&StreamDeliveryBody::new(events)).map_err(std::io::Error::other)?;
    if bytes.len() > MAX_STREAM_DELIVERY_BYTES {
        return Err(std::io::Error::other("stream delivery is too large"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_id_policy_is_bounded_and_url_safe() {
        assert!(is_valid_subscription_id("main-window_01"));
        assert!(!is_valid_subscription_id(""));
        assert!(!is_valid_subscription_id("contains:colon"));
        assert!(!is_valid_subscription_id(&"a".repeat(65)));
    }

    #[test]
    fn delivery_size_check_enforces_the_contract_limit() {
        assert!(delivery_fits(&StreamDeliveryBody::new(vec!["small"])));
        assert!(!delivery_fits(&StreamDeliveryBody::new(vec![
            "x".repeat(MAX_STREAM_DELIVERY_BYTES)
        ])));
    }
}
