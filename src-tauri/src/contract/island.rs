use std::time::Instant;

use serde::Serialize;

use crate::state::AppState;

pub(crate) const ISLAND_CONTRACT_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IslandSnapshot {
    pub contract_version: u32,
    pub enabled: bool,
    pub notice: Option<IslandNoticeSnapshot>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IslandNoticeSnapshot {
    pub id: String,
    pub tone: &'static str,
    pub message_key: &'static str,
    pub message_arguments: Vec<String>,
    pub undo_available: bool,
    pub remaining_ms: u64,
}

impl IslandSnapshot {
    pub(crate) fn from_state(state: &AppState) -> Self {
        let enabled = state.ui_config_snapshot().island_notifications;
        let notice = enabled
            .then(|| state.island_notice())
            .flatten()
            .map(|notice| IslandNoticeSnapshot {
                id: notice.id,
                tone: notice.tone,
                message_key: notice.message_key,
                message_arguments: notice.message_arguments,
                undo_available: notice.undo_token.is_some(),
                remaining_ms: notice
                    .expires_at
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64,
            });
        Self {
            contract_version: ISLAND_CONTRACT_VERSION,
            enabled,
            notice,
        }
    }
}
