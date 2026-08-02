//! Compatibility adapter for the migration-period egui diagnostics page.

use crate::{
    core::diagnostics as core,
    storage::i18n::{t, tf},
};

pub use core::{DiagnosticSnapshot, DiagnosticStatus};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiagnosticCheck {
    pub status: DiagnosticStatus,
    pub title: String,
    pub detail: String,
    pub suggestion: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiagnosticReport {
    pub checks: Vec<DiagnosticCheck>,
}

impl DiagnosticReport {
    pub fn failed_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|check| check.status == DiagnosticStatus::Failed)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|check| check.status == DiagnosticStatus::Warning)
            .count()
    }

    pub fn redacted_text(&self) -> String {
        let mut text = String::new();
        text.push_str(&t("NTE DPS TOOL auto-diagnostics report"));
        text.push('\n');
        text.push_str(&tf(
            "Failed {}, warnings {}",
            &[
                &self.failed_count().to_string(),
                &self.warning_count().to_string(),
            ],
        ));
        text.push('\n');
        for check in &self.checks {
            text.push_str(&format!(
                "[{}] {} - {}\n",
                t(check.status.label_key()),
                check.title,
                check.suggestion
            ));
        }
        text.trim_end().to_owned()
    }
}

pub fn run_capture_diagnostics(snapshot: DiagnosticSnapshot) -> DiagnosticReport {
    let run = core::run_capture_diagnostics(snapshot);
    DiagnosticReport {
        checks: run
            .report
            .checks
            .into_iter()
            .map(|check| DiagnosticCheck {
                status: check.status,
                title: t(check.title_key),
                detail: localize(check.detail.key, &check.detail.arguments),
                suggestion: localize(check.suggestion.key, &check.suggestion.arguments),
            })
            .collect(),
    }
}

fn localize(key: &str, arguments: &[String]) -> String {
    let arguments = arguments.iter().map(String::as_str).collect::<Vec<_>>();
    tf(key, &arguments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacted_report_omits_details_that_may_contain_local_state() {
        let report = DiagnosticReport {
            checks: vec![DiagnosticCheck {
                status: DiagnosticStatus::Failed,
                title: t("Game Connection"),
                detail: r#"IP 192.168.1.2 GUID \Device\NPF_{abc} path C:\Users\me"#.to_owned(),
                suggestion: t("Address the failed items above, then re-detect"),
            }],
        };

        let text = report.redacted_text();

        assert!(text.contains(&t("Address the failed items above, then re-detect")));
        assert!(!text.contains("192.168.1.2"));
        assert!(!text.contains("NPF_"));
        assert!(!text.contains("C:\\Users"));
    }
}
