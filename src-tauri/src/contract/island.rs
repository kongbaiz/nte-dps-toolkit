use std::time::Instant;

use serde::Serialize;
use ts_rs::TS;

use crate::state::AppState;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub(crate) enum IslandTone {
    Info,
    Success,
    Error,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub(crate) struct IslandSnapshot {
    pub enabled: bool,
    pub notice: Option<IslandNoticeSnapshot>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub(crate) struct IslandNoticeSnapshot {
    pub id: String,
    pub tone: IslandTone,
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
        Self { enabled, notice }
    }
}
