use serde::Serialize;

pub(crate) const STREAM_PROTOCOL_VERSION: u32 = 1;
pub(crate) const MAX_EVENTS_PER_STREAM_DELIVERY: usize = 2;
pub(crate) const MAX_STREAM_DELIVERY_BYTES: usize = 14 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum StreamKind {
    Technical,
    Diagnostics,
    History,
    MainDps,
    MainDpsDetail,
    EmptyCurtain,
    ModStudioRuntime,
    Packets,
    Settings,
    Skills,
    Timeline,
}

impl StreamKind {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::Technical => "technical",
            Self::Diagnostics => "diagnostics",
            Self::History => "history",
            Self::MainDps => "mainDps",
            Self::MainDpsDetail => "mainDpsDetail",
            Self::EmptyCurtain => "emptyCurtain",
            Self::ModStudioRuntime => "modStudioRuntime",
            Self::Packets => "packets",
            Self::Settings => "settings",
            Self::Skills => "skills",
            Self::Timeline => "timeline",
        }
    }

    #[cfg(test)]
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "technical" => Some(Self::Technical),
            "diagnostics" => Some(Self::Diagnostics),
            "history" => Some(Self::History),
            "mainDps" => Some(Self::MainDps),
            "mainDpsDetail" => Some(Self::MainDpsDetail),
            "emptyCurtain" => Some(Self::EmptyCurtain),
            "modStudioRuntime" => Some(Self::ModStudioRuntime),
            "packets" => Some(Self::Packets),
            "settings" => Some(Self::Settings),
            "skills" => Some(Self::Skills),
            "timeline" => Some(Self::Timeline),
            _ => None,
        }
    }

    pub(crate) fn stream_key(self, subscription_id: &str) -> String {
        format!("{}:{subscription_id}", self.code())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StreamDeliveryBody<T> {
    pub stream_protocol_version: u32,
    pub events: Vec<T>,
}

impl<T> StreamDeliveryBody<T> {
    pub(crate) fn new(events: Vec<T>) -> Self {
        Self {
            stream_protocol_version: STREAM_PROTOCOL_VERSION,
            events,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_kind_round_trips_stable_protocol_codes_and_keys() {
        let kinds = [
            StreamKind::Technical,
            StreamKind::Diagnostics,
            StreamKind::History,
            StreamKind::MainDps,
            StreamKind::MainDpsDetail,
            StreamKind::EmptyCurtain,
            StreamKind::ModStudioRuntime,
            StreamKind::Packets,
            StreamKind::Settings,
            StreamKind::Skills,
            StreamKind::Timeline,
        ];

        for kind in kinds {
            assert_eq!(StreamKind::parse(kind.code()), Some(kind));
            assert_eq!(
                kind.stream_key("subscription"),
                format!("{}:subscription", kind.code())
            );
        }
        assert_eq!(StreamKind::parse("unknown"), None);
    }

    #[test]
    fn stream_delivery_body_preserves_event_order() {
        let body = StreamDeliveryBody::new(vec!["connection", "batch"]);
        let value = serde_json::to_value(body).expect("serialize stream delivery body");

        assert_eq!(value["streamProtocolVersion"], STREAM_PROTOCOL_VERSION);
        assert_eq!(value["events"], serde_json::json!(["connection", "batch"]));
    }
}
