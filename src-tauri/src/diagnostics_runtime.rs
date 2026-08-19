use std::sync::{Mutex, MutexGuard};

use nte_dps_tool::core::diagnostics::DiagnosticRun;

#[derive(Debug, Default)]
struct DiagnosticsState {
    report: Option<DiagnosticRun>,
    revision: u64,
}

/// Ephemeral diagnostic output and its publication revision.
///
/// A poisoned update may have left a partially published report. The first
/// caller after poison discards it and clears the poison flag. If that discard
/// changes the published state, it advances the revision exactly once.
#[derive(Debug, Default)]
pub(crate) struct DiagnosticsRuntime {
    state: Mutex<DiagnosticsState>,
}

impl DiagnosticsRuntime {
    pub(crate) fn revision(&self) -> u64 {
        self.lock_recover().0.revision
    }

    pub(crate) fn snapshot(&self) -> (u64, Option<DiagnosticRun>) {
        let (state, _) = self.lock_recover();
        (state.revision, state.report.clone())
    }

    /// Stores one complete diagnostic run. Equal output is a semantic no-op.
    pub(crate) fn store(&self, report: DiagnosticRun) -> bool {
        let (mut state, recovery_bumped) = self.lock_recover();
        if state.report.as_ref() == Some(&report) {
            return false;
        }
        state.report = Some(report);
        if !recovery_bumped {
            state.revision = next_revision(state.revision);
        }
        true
    }

    fn lock_recover(&self) -> (MutexGuard<'_, DiagnosticsState>, bool) {
        match self.state.lock() {
            Ok(state) => (state, false),
            Err(poison) => {
                let mut state = poison.into_inner();
                let changed = state.report.take().is_some();
                if changed {
                    state.revision = next_revision(state.revision);
                }
                self.state.clear_poison();
                (state, changed)
            }
        }
    }
}

const fn next_revision(revision: u64) -> u64 {
    revision.wrapping_add(1)
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::*;

    #[test]
    fn poisoned_report_is_discarded_and_revision_advances_once() {
        let runtime = DiagnosticsRuntime::default();
        assert!(runtime.store(DiagnosticRun::default()));
        assert_eq!(runtime.revision(), 1);

        let poisoned = catch_unwind(AssertUnwindSafe(|| {
            let mut state = runtime.state.lock().expect("diagnostics state locks");
            state.report = Some(DiagnosticRun::default());
            panic!("poison diagnostics state after a partial update");
        }));
        assert!(poisoned.is_err());

        assert_eq!(runtime.snapshot(), (2, None));
        assert_eq!(runtime.snapshot(), (2, None));
        assert_eq!(runtime.revision(), 2);
    }

    #[test]
    fn equal_report_is_a_noop_and_changed_report_bumps_once() {
        let runtime = DiagnosticsRuntime::default();
        let report = DiagnosticRun::default();
        assert!(runtime.store(report.clone()));
        assert!(!runtime.store(report));
        assert_eq!(runtime.revision(), 1);

        let mut changed = DiagnosticRun::default();
        changed.environment.manual_device = true;
        assert!(runtime.store(changed.clone()));
        assert_eq!(runtime.snapshot(), (2, Some(changed)));
    }

    #[test]
    fn a_complete_store_coalesces_poison_recovery_into_one_revision() {
        let runtime = DiagnosticsRuntime::default();
        let poisoned = catch_unwind(AssertUnwindSafe(|| {
            let _state = runtime.state.lock().expect("diagnostics state locks");
            panic!("poison diagnostics state");
        }));
        assert!(poisoned.is_err());

        let mut report = DiagnosticRun::default();
        report.environment.manual_device = true;
        assert!(runtime.store(report.clone()));
        assert_eq!(runtime.snapshot(), (1, Some(report)));
    }
}
