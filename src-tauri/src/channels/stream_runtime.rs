use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::atomic::Ordering,
    time::Duration,
};

use serde::Serialize;
use tauri::{State, WebviewWindow, ipc::Channel};
use tokio::time::{MissedTickBehavior, interval};

use crate::{
    contract::{
        CommandError,
        stream::{
            MAX_EVENTS_PER_STREAM_DELIVERY, MAX_STREAM_DELIVERY_BYTES, StreamDeliveryBody,
            StreamKind,
        },
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

// The receipt binds generation; the ordered Channel binds delivery sequence.
#[tauri::command]
pub(crate) fn acknowledge_stream_delivery(
    subscription_id: String,
    stream_kind: String,
    stream_generation: String,
    delivery_sequence: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    validate_subscription_id(&subscription_id)?;
    let kind = StreamKind::parse(&stream_kind).ok_or_else(CommandError::invalid_subscription_id)?;
    let generation = parse_delivery_counter(&stream_generation)?;
    let sequence = parse_delivery_counter(&delivery_sequence)?;
    state
        .acknowledge_stream(
            window.label(),
            &kind.stream_key(&subscription_id),
            generation,
            sequence,
        )
        .map(|_| ())
        .map_err(stream_registry_error)
}

fn parse_delivery_counter(value: &str) -> Result<u64, CommandError> {
    if value.is_empty()
        || value.len() > 20
        || value.starts_with('0')
        || !value.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(CommandError::invalid_subscription_id());
    }
    value
        .parse()
        .map_err(|_| CommandError::invalid_subscription_id())
}

/// Result of one revision-aware projection pass.
pub(crate) enum PollingStreamOutput<T> {
    NoChange,
    Event(T),
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
            if stop.load(Ordering::Acquire)
                || !active.load(Ordering::Acquire)
                || !task_registration.delivery_ready()
            {
                continue;
            }

            let output = catch_unwind(AssertUnwindSafe(|| poll(&task_state)));
            let events = match output {
                Ok(PollingStreamOutput::NoChange) => continue,
                Ok(PollingStreamOutput::Event(event)) => vec![event],
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
            if !task_registration.begin_delivery() || delivery.channel.send(body).is_err() {
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

/// Subscription-owned blocking work; cancellation is passed into finite IPC.
/// Unlike projection-only streams, this must not occupy a Tokio executor thread.
pub(crate) fn spawn_blocking_polling_stream<T, Poll>(
    delivery: StreamDeliveryEndpoint<T>,
    state: AppState,
    registration: StreamRegistration,
    interval_ms: u32,
    mut poll: Poll,
) -> Result<(), CommandError>
where
    T: Serialize + Send + 'static,
    Poll: FnMut(&AppState, &std::sync::Arc<std::sync::atomic::AtomicBool>) -> PollingStreamOutput<T>
        + Send
        + 'static,
{
    match state.activate_stream(&registration) {
        Ok(true) => {}
        _ => {
            registration.cancel();
            let _ = state.finish_stream(&registration);
            return Err(CommandError::stream_runtime_unavailable());
        }
    }
    tauri::async_runtime::spawn_blocking(move || {
        let stop = registration.stop_token();
        let active = registration.activation_token();
        while !stop.load(Ordering::Acquire) {
            if active.load(Ordering::Acquire) && registration.delivery_ready() {
                let output = catch_unwind(AssertUnwindSafe(|| poll(&state, &stop)));
                match output {
                    Ok(PollingStreamOutput::NoChange) => {}
                    Ok(PollingStreamOutput::Event(event)) => {
                        let body = StreamDeliveryBody::new(vec![event]);
                        if stop.load(Ordering::Acquire)
                            || !delivery_fits(&body)
                            || !registration.begin_delivery()
                            || delivery.channel.send(body).is_err()
                        {
                            break;
                        }
                    }
                    _ => break,
                }
            }
            for _ in 0..interval_ms.div_ceil(25) {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
        registration.cancel();
        let _ = state.finish_stream(&registration);
    });
    Ok(())
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
    fn failed_delivery_cancels_and_removes_both_polling_worker_kinds() {
        struct Dropped(std::sync::mpsc::Sender<()>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        for blocking in [false, true] {
            let state = AppState::default();
            let registration = state.reserve_stream("console", "timeline:failure").unwrap();
            let stop = registration.stop_token();
            let (sender, receiver) = std::sync::mpsc::channel();
            let dropped = Dropped(sender);
            let channel = Channel::new(move |_| {
                let _ = &dropped;
                Err(std::io::Error::other("fixture disconnect").into())
            });
            let delivery = StreamDeliveryEndpoint::new(channel);
            if blocking {
                spawn_blocking_polling_stream(delivery, state.clone(), registration, 1, |_, _| {
                    PollingStreamOutput::Event("snapshot")
                })
                .unwrap();
            } else {
                spawn_polling_stream("fixture", delivery, state.clone(), registration, 1, |_| {
                    PollingStreamOutput::Event("snapshot")
                })
                .unwrap();
            }
            receiver
                .recv_timeout(Duration::from_secs(10))
                .expect("worker released channel");
            assert!(stop.load(Ordering::Acquire));
            assert_eq!(state.stream_registry_len().unwrap(), 0);
        }
    }

    #[test]
    fn delivery_counters_reject_noncanonical_and_overflowing_input() {
        for value in ["", "0", "01", "-1", "+1", "1.0", "18446744073709551616"] {
            assert!(parse_delivery_counter(value).is_err(), "{value}");
        }
        assert_eq!(parse_delivery_counter("1").unwrap(), 1);
        assert_eq!(
            parse_delivery_counter("18446744073709551615").unwrap(),
            u64::MAX
        );
    }

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
