use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub(crate) const MAX_STREAM_DELIVERY_BYTES: usize = 14 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) enum StreamKind {
    Technical,
    Diagnostics,
    History,
    MainDps,
    MainDpsDetail,
    EmptyCurtain,
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
            Self::Packets => "packets",
            Self::Settings => "settings",
            Self::Skills => "skills",
            Self::Timeline => "timeline",
        }
    }

    pub(crate) fn stream_key(self, subscription_id: &str) -> String {
        format!("{}:{subscription_id}", self.code())
    }
}
