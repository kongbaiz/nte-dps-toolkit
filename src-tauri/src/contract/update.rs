use serde::Serialize;

use super::settings::UpdateSettingsSnapshot;

pub(crate) const UPDATE_PROMPT_CONTRACT_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdatePromptSnapshot {
    pub contract_version: u32,
    pub updates: UpdateSettingsSnapshot,
}

impl UpdatePromptSnapshot {
    pub(crate) fn from_updates(updates: UpdateSettingsSnapshot) -> Self {
        Self {
            contract_version: UPDATE_PROMPT_CONTRACT_VERSION,
            updates,
        }
    }
}
