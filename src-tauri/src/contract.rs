use serde::Serialize;

pub(crate) const TECHNICAL_CONTRACT_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TechnicalSnapshot {
    pub contract_version: u32,
    pub sequence: String,
    pub bridge_status: &'static str,
    pub adapter_version: &'static str,
    pub window_label: &'static str,
    pub uptime_ms: String,
    pub stream_interval_ms: u32,
    pub supported_locales: Vec<&'static str>,
    pub window: HudWindowSnapshot,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HudWindowSnapshot {
    pub passthrough: bool,
    pub always_on_top: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", content = "payload", rename_all = "camelCase")]
pub(crate) enum TechnicalEvent {
    Snapshot(TechnicalSnapshot),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubscriptionReceipt {
    pub subscription_id: String,
    pub stream_interval_ms: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommandError {
    pub code: &'static str,
    pub message_key: &'static str,
    pub message_arguments: Vec<String>,
}

impl CommandError {
    pub(crate) fn invalid_subscription_id() -> Self {
        Self {
            code: "invalid_subscription_id",
            message_key: "Technical subscription identifier is invalid.",
            message_arguments: Vec::new(),
        }
    }

    pub(crate) fn invalid_window() -> Self {
        Self {
            code: "invalid_window",
            message_key: "Technical command is not available for this window.",
            message_arguments: Vec::new(),
        }
    }

    pub(crate) fn window_operation_failed() -> Self {
        Self {
            code: "window_operation_failed",
            message_key: "HUD window operation failed.",
            message_arguments: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn technical_snapshot_uses_camel_case_and_string_sequence() {
        let snapshot = TechnicalSnapshot {
            contract_version: 1,
            sequence: "9007199254740992".to_owned(),
            bridge_status: "ready",
            adapter_version: "0.3.6",
            window_label: "hud-spike",
            uptime_ms: "12".to_owned(),
            stream_interval_ms: 750,
            supported_locales: vec!["en", "zh-CN"],
            window: HudWindowSnapshot {
                passthrough: false,
                always_on_top: true,
            },
        };

        let value = serde_json::to_value(snapshot).expect("snapshot must serialize");

        assert_eq!(value["contractVersion"], 1);
        assert_eq!(value["sequence"], "9007199254740992");
        assert_eq!(value["window"]["alwaysOnTop"], true);
        assert!(value.get("contract_version").is_none());
    }

    #[test]
    fn technical_event_has_stable_discriminant() {
        let event = TechnicalEvent::Snapshot(TechnicalSnapshot {
            contract_version: 1,
            sequence: "1".to_owned(),
            bridge_status: "ready",
            adapter_version: "0.3.6",
            window_label: "hud-spike",
            uptime_ms: "12".to_owned(),
            stream_interval_ms: 750,
            supported_locales: vec!["en"],
            window: HudWindowSnapshot {
                passthrough: false,
                always_on_top: true,
            },
        });

        let value = serde_json::to_value(event).expect("event must serialize");

        assert_eq!(value["event"], "snapshot");
        assert_eq!(value["payload"]["contractVersion"], 1);
    }
}
