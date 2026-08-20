use serde::Serialize;

use nte_dps_tool::{engine::model::CombatClockRuntimeHealth, storage::config::DpsTimeMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DpsTimeRuntimeSnapshot {
    pub configured_mode: &'static str,
    pub effective_mode: &'static str,
    pub combat_clock_health: &'static str,
    pub degraded: bool,
    pub warning_message_key: Option<&'static str>,
}

impl DpsTimeRuntimeSnapshot {
    pub(crate) fn new(configured: DpsTimeMode, health: CombatClockRuntimeHealth) -> Self {
        let configured_mode = mode_id(configured);
        if configured == DpsTimeMode::RealTime {
            return Self {
                configured_mode,
                effective_mode: mode_id(DpsTimeMode::RealTime),
                combat_clock_health: health_id(health),
                degraded: false,
                warning_message_key: None,
            };
        }

        let (effective_mode, warning_message_key) = match health {
            CombatClockRuntimeHealth::Available | CombatClockRuntimeHealth::Recorded => {
                (mode_id(DpsTimeMode::TimeStopAdjusted), None)
            }
            CombatClockRuntimeHealth::Unknown => (
                mode_id(DpsTimeMode::RealTime),
                Some("Time-stop adjustment has not been verified for this session."),
            ),
            CombatClockRuntimeHealth::ProviderUnavailable => (
                mode_id(DpsTimeMode::RealTime),
                Some(
                    "Time-stop adjustment is unavailable because the combat-clock provider is not connected.",
                ),
            ),
            CombatClockRuntimeHealth::ModDisabled => (
                mode_id(DpsTimeMode::RealTime),
                Some("Time-stop adjustment is unavailable because the required Mod is disabled."),
            ),
            CombatClockRuntimeHealth::DataUnavailable => (
                mode_id(DpsTimeMode::RealTime),
                Some(
                    "Time-stop adjustment is unavailable because the combat-clock provider has no authoritative pause state.",
                ),
            ),
            CombatClockRuntimeHealth::InvalidResponse => (
                mode_id(DpsTimeMode::RealTime),
                Some(
                    "Time-stop adjustment is unavailable because the combat-clock response is invalid.",
                ),
            ),
        };
        Self {
            configured_mode,
            effective_mode,
            combat_clock_health: health_id(health),
            degraded: warning_message_key.is_some(),
            warning_message_key,
        }
    }
}

pub(crate) const fn mode_id(mode: DpsTimeMode) -> &'static str {
    match mode {
        DpsTimeMode::TimeStopAdjusted => "time-stop-adjusted",
        DpsTimeMode::RealTime => "real-time",
    }
}

const fn health_id(health: CombatClockRuntimeHealth) -> &'static str {
    match health {
        CombatClockRuntimeHealth::Unknown => "unknown",
        CombatClockRuntimeHealth::Available => "available",
        CombatClockRuntimeHealth::Recorded => "recorded",
        CombatClockRuntimeHealth::ProviderUnavailable => "provider-unavailable",
        CombatClockRuntimeHealth::ModDisabled => "mod-disabled",
        CombatClockRuntimeHealth::DataUnavailable => "data-unavailable",
        CombatClockRuntimeHealth::InvalidResponse => "invalid-response",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_adjustment_never_claims_effective_without_provider_health() {
        let unknown = DpsTimeRuntimeSnapshot::new(
            DpsTimeMode::TimeStopAdjusted,
            CombatClockRuntimeHealth::Unknown,
        );
        assert_eq!(unknown.configured_mode, "time-stop-adjusted");
        assert_eq!(unknown.effective_mode, "real-time");
        assert!(unknown.degraded);
        assert!(unknown.warning_message_key.is_some());

        let available = DpsTimeRuntimeSnapshot::new(
            DpsTimeMode::TimeStopAdjusted,
            CombatClockRuntimeHealth::Available,
        );
        assert_eq!(available.effective_mode, "time-stop-adjusted");
        assert!(!available.degraded);
        assert!(available.warning_message_key.is_none());

        let data_unavailable = DpsTimeRuntimeSnapshot::new(
            DpsTimeMode::TimeStopAdjusted,
            CombatClockRuntimeHealth::DataUnavailable,
        );
        assert_eq!(data_unavailable.effective_mode, "real-time");
        assert_eq!(data_unavailable.combat_clock_health, "data-unavailable");
        assert!(data_unavailable.degraded);
        assert_eq!(
            data_unavailable.warning_message_key,
            Some(
                "Time-stop adjustment is unavailable because the combat-clock provider has no authoritative pause state."
            )
        );

        let real_time = DpsTimeRuntimeSnapshot::new(
            DpsTimeMode::RealTime,
            CombatClockRuntimeHealth::ProviderUnavailable,
        );
        assert_eq!(real_time.effective_mode, "real-time");
        assert!(!real_time.degraded);
        assert!(real_time.warning_message_key.is_none());
    }
}
