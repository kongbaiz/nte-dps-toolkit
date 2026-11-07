use serde::Serialize;
use ts_rs::TS;

use super::settings::UpdateSettingsSnapshot;

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdatePromptSnapshot {
    pub updates: UpdateSettingsSnapshot,
}

impl UpdatePromptSnapshot {
    pub(crate) fn from_updates(updates: UpdateSettingsSnapshot) -> Self {
        Self { updates }
    }
}
