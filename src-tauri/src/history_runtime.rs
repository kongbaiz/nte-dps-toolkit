use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crate::state::AppState;

const HISTORY_MAINTENANCE_INTERVAL: Duration = Duration::from_millis(250);

pub(crate) struct HistoryRuntime {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl HistoryRuntime {
    pub(crate) fn start(state: AppState) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("nte-history-maintenance".to_owned())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    state.maintain_history_rounds();
                    thread::sleep(HISTORY_MAINTENANCE_INTERVAL);
                }
            })?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for HistoryRuntime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
