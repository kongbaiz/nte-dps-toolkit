use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

use nte_dps_tool::storage::i18n::Language;

use crate::{
    contract::{HudWindowSnapshot, TECHNICAL_CONTRACT_VERSION, TechnicalSnapshot},
    windows::hud::HUD_WINDOW_LABEL,
};

pub(crate) const TECHNICAL_STREAM_INTERVAL_MS: u32 = 750;

#[derive(Clone)]
pub(crate) struct AppState(Arc<AppStateInner>);

struct AppStateInner {
    started_at: Instant,
    sequence: AtomicU64,
    passthrough: AtomicBool,
    always_on_top: AtomicBool,
    streams: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self(Arc::new(AppStateInner {
            started_at: Instant::now(),
            sequence: AtomicU64::new(0),
            passthrough: AtomicBool::new(false),
            always_on_top: AtomicBool::new(true),
            streams: Mutex::new(HashMap::new()),
        }))
    }
}

impl AppState {
    pub(crate) fn snapshot(&self) -> TechnicalSnapshot {
        let sequence = self.0.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let supported_locales = Language::all()
            .iter()
            .map(|language| language.code())
            .collect();

        TechnicalSnapshot {
            contract_version: TECHNICAL_CONTRACT_VERSION,
            sequence: sequence.to_string(),
            bridge_status: "ready",
            adapter_version: env!("CARGO_PKG_VERSION"),
            window_label: HUD_WINDOW_LABEL,
            uptime_ms: self.uptime_ms().to_string(),
            stream_interval_ms: TECHNICAL_STREAM_INTERVAL_MS,
            supported_locales,
            window: HudWindowSnapshot {
                passthrough: self.passthrough(),
                always_on_top: self.always_on_top(),
            },
        }
    }

    pub(crate) fn passthrough(&self) -> bool {
        self.0.passthrough.load(Ordering::Acquire)
    }

    pub(crate) fn set_passthrough(&self, enabled: bool) {
        self.0.passthrough.store(enabled, Ordering::Release);
    }

    pub(crate) fn always_on_top(&self) -> bool {
        self.0.always_on_top.load(Ordering::Acquire)
    }

    pub(crate) fn set_always_on_top(&self, enabled: bool) {
        self.0.always_on_top.store(enabled, Ordering::Release);
    }

    pub(crate) fn uptime_ms(&self) -> u128 {
        self.0.started_at.elapsed().as_millis()
    }

    pub(crate) fn begin_stream(&self, subscription_id: String) -> Arc<AtomicBool> {
        let stop = Arc::new(AtomicBool::new(false));
        let replaced = self
            .0
            .streams
            .lock()
            .expect("technical stream registry lock poisoned")
            .insert(subscription_id, Arc::clone(&stop));

        if let Some(replaced) = replaced {
            replaced.store(true, Ordering::Release);
        }

        stop
    }

    pub(crate) fn stop_stream(&self, subscription_id: &str) {
        if let Some(stop) = self
            .0
            .streams
            .lock()
            .expect("technical stream registry lock poisoned")
            .remove(subscription_id)
        {
            stop.store(true, Ordering::Release);
        }
    }

    pub(crate) fn finish_stream(&self, subscription_id: &str, stop: &Arc<AtomicBool>) {
        let mut streams = self
            .0
            .streams
            .lock()
            .expect("technical stream registry lock poisoned");
        let is_current = streams
            .get(subscription_id)
            .is_some_and(|current| Arc::ptr_eq(current, stop));

        if is_current {
            streams.remove(subscription_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_subscription_stops_previous_stream() {
        let state = AppState::default();
        let previous = state.begin_stream("technical".to_owned());
        let current = state.begin_stream("technical".to_owned());

        assert!(previous.load(Ordering::Acquire));
        assert!(!current.load(Ordering::Acquire));
    }

    #[test]
    fn finishing_replaced_stream_keeps_current_registration() {
        let state = AppState::default();
        let previous = state.begin_stream("technical".to_owned());
        let current = state.begin_stream("technical".to_owned());

        state.finish_stream("technical", &previous);
        state.stop_stream("technical");

        assert!(current.load(Ordering::Acquire));
    }
}
