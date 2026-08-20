use std::{
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Instant,
};

/// Owns process-lifetime desktop shell scalars. Grouping these atomics keeps
/// `AppState` a composition facade instead of a flat global-state container.
pub(crate) struct DesktopRuntime {
    started_at: Instant,
    sequence: AtomicU64,
    passthrough: AtomicBool,
    passthrough_hotkey_ready: AtomicBool,
    hud_always_on_top: AtomicBool,
    settings_revision: AtomicU64,
    onboarding_step: AtomicU64,
}

impl DesktopRuntime {
    pub(crate) fn new(hud_always_on_top: bool) -> Self {
        Self {
            started_at: Instant::now(),
            sequence: AtomicU64::new(0),
            passthrough: AtomicBool::new(false),
            passthrough_hotkey_ready: AtomicBool::new(false),
            hud_always_on_top: AtomicBool::new(hud_always_on_top),
            settings_revision: AtomicU64::new(0),
            onboarding_step: AtomicU64::new(0),
        }
    }

    pub(crate) fn next_sequence(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub(crate) fn uptime_ms(&self) -> u128 {
        self.started_at.elapsed().as_millis()
    }

    pub(crate) fn passthrough(&self) -> bool {
        self.passthrough.load(Ordering::Acquire)
    }

    pub(crate) fn set_passthrough(&self, enabled: bool) -> bool {
        self.passthrough.swap(enabled, Ordering::AcqRel) != enabled
    }

    pub(crate) fn passthrough_hotkey_ready(&self) -> bool {
        self.passthrough_hotkey_ready.load(Ordering::Acquire)
    }

    pub(crate) fn set_passthrough_hotkey_ready(&self, ready: bool) {
        self.passthrough_hotkey_ready
            .store(ready, Ordering::Release);
    }

    pub(crate) fn hud_always_on_top(&self) -> bool {
        self.hud_always_on_top.load(Ordering::Acquire)
    }

    pub(crate) fn set_hud_always_on_top(&self, enabled: bool) {
        self.hud_always_on_top.store(enabled, Ordering::Release);
    }

    pub(crate) fn settings_revision(&self) -> u64 {
        self.settings_revision.load(Ordering::Acquire)
    }

    pub(crate) fn bump_settings_revision(&self) -> u64 {
        self.settings_revision.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub(crate) fn set_onboarding_step(&self, step: usize) {
        self.onboarding_step
            .store(step.min(3) as u64, Ordering::Release);
    }

    pub(crate) fn onboarding_step(&self) -> usize {
        self.onboarding_step.load(Ordering::Acquire).min(3) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_mutations_report_real_state_changes() {
        let runtime = DesktopRuntime::new(true);
        assert_eq!(runtime.next_sequence(), 1);
        assert_eq!(runtime.next_sequence(), 2);
        assert!(runtime.hud_always_on_top());
        assert!(!runtime.passthrough());
        assert!(runtime.set_passthrough(true));
        assert!(!runtime.set_passthrough(true));
        runtime.set_onboarding_step(usize::MAX);
        assert_eq!(runtime.onboarding_step(), 3);
        assert_eq!(runtime.bump_settings_revision(), 1);
        assert_eq!(runtime.settings_revision(), 1);
    }
}
