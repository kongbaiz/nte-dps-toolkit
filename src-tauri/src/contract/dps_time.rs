use serde::Serialize;
use ts_rs::TS;

use nte_dps_tool::{engine::model::CombatClockRuntimeHealth, storage::config::DpsTimeMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EffectiveDpsTimeMode {
    TimeStopAdjusted,
    RealTime,
    /// No hits yet and the plugin clock has not reported; not a wall-clock fallback.
    Pending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DpsTimeRuntimeSnapshot {
    pub configured_mode: DpsTimeMode,
    pub effective_mode: EffectiveDpsTimeMode,
    pub combat_clock_health: CombatClockRuntimeHealth,
    pub degraded: bool,
    pub warning_message_key: Option<&'static str>,
}

impl DpsTimeRuntimeSnapshot {
    /// With no measured hits, an uninitialized plugin clock is pending, not a
    /// wall-clock fallback. Real capture errors are still exposed by CaptureSnapshot.
    pub(crate) fn for_readout(
        configured: DpsTimeMode,
        health: CombatClockRuntimeHealth,
        empty: bool,
    ) -> Self {
        if empty
            && configured == DpsTimeMode::TimeStopAdjusted
            && !health.supports_time_stop_adjustment()
        {
            return Self {
                configured_mode: configured,
                effective_mode: EffectiveDpsTimeMode::Pending,
                combat_clock_health: health,
                degraded: false,
                warning_message_key: None,
            };
        }
        Self::new(configured, health)
    }

    pub(crate) fn new(configured: DpsTimeMode, health: CombatClockRuntimeHealth) -> Self {
        if configured == DpsTimeMode::RealTime {
            return Self {
                configured_mode: configured,
                effective_mode: EffectiveDpsTimeMode::RealTime,
                combat_clock_health: health,
                degraded: false,
                warning_message_key: None,
            };
        }

        let (effective_mode, warning_message_key) = match health {
            CombatClockRuntimeHealth::Available | CombatClockRuntimeHealth::Recorded => {
                (EffectiveDpsTimeMode::TimeStopAdjusted, None)
            }
            CombatClockRuntimeHealth::Unknown => (
                EffectiveDpsTimeMode::RealTime,
                Some("Time-stop adjustment has not been verified for this session."),
            ),
            CombatClockRuntimeHealth::DataUnavailable => (
                EffectiveDpsTimeMode::RealTime,
                Some(
                    "Time-stop adjustment is unavailable because the combat-clock provider has no authoritative pause state.",
                ),
            ),
        };
        Self {
            configured_mode: configured,
            effective_mode,
            combat_clock_health: health,
            degraded: warning_message_key.is_some(),
            warning_message_key,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_plugin_readout_waits_for_plugin_time_without_a_wall_clock_fallback() {
        let waiting = DpsTimeRuntimeSnapshot::for_readout(
            DpsTimeMode::TimeStopAdjusted,
            CombatClockRuntimeHealth::Unknown,
            true,
        );
        assert_eq!(waiting.effective_mode, EffectiveDpsTimeMode::Pending);
        assert!(!waiting.degraded);
        assert!(waiting.warning_message_key.is_none());
        let ready = DpsTimeRuntimeSnapshot::for_readout(
            DpsTimeMode::TimeStopAdjusted,
            CombatClockRuntimeHealth::Available,
            false,
        );
        assert_eq!(ready.effective_mode, EffectiveDpsTimeMode::TimeStopAdjusted);
        let packet = DpsTimeRuntimeSnapshot::for_readout(
            DpsTimeMode::RealTime,
            CombatClockRuntimeHealth::Unknown,
            true,
        );
        assert_eq!(packet.effective_mode, EffectiveDpsTimeMode::RealTime);
    }

    #[test]
    fn configured_adjustment_never_claims_effective_without_provider_health() {
        let unknown = DpsTimeRuntimeSnapshot::new(
            DpsTimeMode::TimeStopAdjusted,
            CombatClockRuntimeHealth::Unknown,
        );
        assert_eq!(unknown.effective_mode, EffectiveDpsTimeMode::RealTime);
        assert!(unknown.degraded);
        assert!(unknown.warning_message_key.is_some());

        let available = DpsTimeRuntimeSnapshot::new(
            DpsTimeMode::TimeStopAdjusted,
            CombatClockRuntimeHealth::Available,
        );
        assert_eq!(
            available.effective_mode,
            EffectiveDpsTimeMode::TimeStopAdjusted
        );
        assert!(!available.degraded);
        assert!(available.warning_message_key.is_none());

        let data_unavailable = DpsTimeRuntimeSnapshot::new(
            DpsTimeMode::TimeStopAdjusted,
            CombatClockRuntimeHealth::DataUnavailable,
        );
        assert_eq!(
            data_unavailable.effective_mode,
            EffectiveDpsTimeMode::RealTime
        );
        assert!(data_unavailable.degraded);
        assert_eq!(
            data_unavailable.warning_message_key,
            Some(
                "Time-stop adjustment is unavailable because the combat-clock provider has no authoritative pause state."
            )
        );

        let real_time = DpsTimeRuntimeSnapshot::new(
            DpsTimeMode::RealTime,
            CombatClockRuntimeHealth::DataUnavailable,
        );
        assert_eq!(real_time.effective_mode, EffectiveDpsTimeMode::RealTime);
        assert!(!real_time.degraded);
        assert!(real_time.warning_message_key.is_none());
    }
}
