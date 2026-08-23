//! Frontend-neutral history archive preparation and round-boundary rules.

use std::time::Duration;

use crate::{
    engine::model::{
        AbyssEvent, CaptureQualitySource, CombatSessionSummary, CombatState, DpsTimeBasis,
    },
    storage::history::HistoryCombatDetails,
};

pub fn abyss_event_starts_new_round(current_floor: Option<u32>, event: &AbyssEvent) -> bool {
    matches!(event, AbyssEvent::RestartDetected { .. })
        || matches!(
            event,
            AbyssEvent::Stage {
                floor: Some(next_floor),
                ..
            } if current_floor.is_some_and(|floor| floor != *next_floor)
        )
}

pub fn auto_round_due(
    capture_running: bool,
    paused: bool,
    abyss_active: bool,
    game_paused: bool,
    has_hits: bool,
    idle_elapsed: Option<Duration>,
    idle_seconds: u32,
) -> bool {
    capture_running
        && !paused
        && !abyss_active
        && !game_paused
        && has_hits
        && idle_elapsed.is_some_and(|elapsed| elapsed >= Duration::from_secs(idle_seconds.into()))
}

#[derive(Clone, Debug)]
pub struct PreparedHistoryArchive {
    pub summary: CombatSessionSummary,
    pub details: Option<HistoryCombatDetails>,
}

/// An Abyss round waiting for the desktop history service to prepare and
/// persist it. The capture source is frozen at the same event-gated boundary
/// as the round details so a later replay/live switch cannot rewrite
/// provenance for an older round.
#[derive(Clone, Debug)]
pub struct PendingHistoryArchive {
    pub summary: CombatSessionSummary,
    pub details: HistoryCombatDetails,
    pub source: CaptureQualitySource,
    /// Effective basis frozen at the event-gated round boundary. It already
    /// accounts for recorded/live provider availability and must not be
    /// recomputed from a later settings snapshot.
    pub dps_time_mode: DpsTimeBasis,
    pub separate_reaction_damage: bool,
}

/// User projection preferences needed when an automatic round boundary is
/// observed inside the capture worker. The worker atomically snapshots this
/// pair and resolves the effective clock basis against the detached state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryArchivePolicy {
    pub requested_dps_time_mode: DpsTimeBasis,
    pub separate_reaction_damage: bool,
}

impl Default for HistoryArchivePolicy {
    fn default() -> Self {
        Self {
            requested_dps_time_mode: DpsTimeBasis::SubtractTimeStop,
            separate_reaction_damage: false,
        }
    }
}

impl HistoryArchivePolicy {
    pub fn effective_for(self, state: &CombatState) -> DpsTimeBasis {
        state.effective_dps_time_basis(self.requested_dps_time_mode)
    }
}

pub fn prepare_history_archive(
    state: &CombatState,
    source: CaptureQualitySource,
    dps_time_mode: DpsTimeBasis,
    separate_reaction_damage: bool,
) -> Option<PreparedHistoryArchive> {
    let details = HistoryCombatDetails::from_state(state);
    state
        .session_summary(source, dps_time_mode, separate_reaction_damage)
        .map(|summary| PreparedHistoryArchive { summary, details })
}

/// Owned counterpart for a round already detached under the capture gate. It
/// computes the bounded summary before moving every hit into History details.
pub fn prepare_history_archive_owned(
    state: CombatState,
    source: CaptureQualitySource,
    dps_time_mode: DpsTimeBasis,
    separate_reaction_damage: bool,
) -> Option<PreparedHistoryArchive> {
    let summary = state.session_summary(source, dps_time_mode, separate_reaction_damage)?;
    let details = HistoryCombatDetails::from_state_owned(state);
    Some(PreparedHistoryArchive { summary, details })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::model::{Hit, HitCharacterSource, HitDirection};

    #[test]
    fn abyss_exit_is_not_a_pre_event_archive_boundary() {
        assert!(!abyss_event_starts_new_round(
            Some(12),
            &AbyssEvent::Exit { timestamp: 10.0 },
        ));
    }

    #[test]
    fn idle_round_requires_running_unpaused_non_abyss_combat() {
        let due = Some(Duration::from_secs(30));
        assert!(auto_round_due(true, false, false, false, true, due, 30));
        assert!(!auto_round_due(false, false, false, false, true, due, 30));
        assert!(!auto_round_due(true, true, false, false, true, due, 30));
        assert!(!auto_round_due(true, false, true, false, true, due, 30));
        assert!(!auto_round_due(true, false, false, true, true, due, 30));
        assert!(!auto_round_due(true, false, false, false, false, due, 30));
    }

    #[test]
    fn empty_state_has_no_archive() {
        assert!(
            prepare_history_archive(
                &CombatState::default(),
                CaptureQualitySource::Live,
                DpsTimeBasis::SubtractTimeStop,
                false,
            )
            .is_none()
        );
    }

    #[test]
    fn archive_keeps_summary_and_bounded_details_from_one_state() {
        let mut state = CombatState::default();
        state.push_hit(Hit {
            timestamp: 10.0,
            char_id: 7,
            char_name: "Test".to_owned(),
            damage: 321.0,
            char_known: true,
            byte_offset: 0,
            bit_shift: 0,
            char_source: HitCharacterSource::Packet,
            direction: HitDirection::Outgoing,
            target_hp_before: 1_000.0,
            target_hp_after: 679.0,
            target_max_hp: 1_000.0,
            max_hp_reduction: 0.0,
            target_hp_percent: 67.9,
            target_id: None,
            target_name: None,
            target_name_en: None,
            target_name_ja: None,
            target_monster_id: None,
            target_context: Vec::new(),
            gameplay_effect_index: None,
            gameplay_effect_name: None,
            ability_name: None,
            damage_name: None,
            damage_component: None,
            attack_type: None,
            damage_attribute: None,
            follow_up_damage: 0.0,
            follow_up_timestamp: None,
            follow_up_damage_name: None,
            follow_up_attack_type: None,
            follow_up_damage_attribute: None,
            reconciled_overkill_damage: None,
            wire_event: None,
        });

        let archive = prepare_history_archive(
            &state,
            CaptureQualitySource::Live,
            DpsTimeBasis::SubtractTimeStop,
            false,
        )
        .expect("combat archive");

        assert_eq!(archive.summary.total_damage, 321.0);
        assert_eq!(
            archive
                .details
                .as_ref()
                .expect("history details")
                .global_hits
                .len(),
            1
        );
    }
}
