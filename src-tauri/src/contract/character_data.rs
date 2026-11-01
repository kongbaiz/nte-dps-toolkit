use serde::Serialize;
use ts_rs::TS;

use nte_dps_tool::core::character_data::{CHARACTER_ATTRIBUTES, CharacterDataProjection};

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CharacterDataSnapshot {
    pub attributes: Vec<&'static str>,
    pub records: Vec<CharacterDataRecordSnapshot>,
}

impl CharacterDataSnapshot {
    pub(crate) fn from_projection(projection: CharacterDataProjection) -> Self {
        Self {
            attributes: CHARACTER_ATTRIBUTES.to_vec(),
            records: projection
                .records
                .into_iter()
                .map(|record| CharacterDataRecordSnapshot {
                    id: record.id,
                    name_zh: record.name_zh,
                    name_en: record.name_en,
                    codename: record.codename,
                    attribute: record.attribute,
                    verified: record.verified,
                    color: record.color,
                    avatar: record.avatar,
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CharacterDataRecordSnapshot {
    pub id: u32,
    pub name_zh: String,
    pub name_en: String,
    pub codename: String,
    pub attribute: String,
    pub verified: bool,
    pub color: String,
    pub avatar: String,
}
