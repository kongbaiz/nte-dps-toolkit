use serde::Serialize;

use nte_dps_tool::core::character_data::{CHARACTER_ATTRIBUTES, CharacterDataProjection};

pub(crate) const CHARACTER_DATA_CONTRACT_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CharacterDataSnapshot {
    pub contract_version: u32,
    pub attributes: Vec<&'static str>,
    pub records: Vec<CharacterDataRecordSnapshot>,
}

impl CharacterDataSnapshot {
    pub(crate) fn from_projection(projection: CharacterDataProjection) -> Self {
        Self {
            contract_version: CHARACTER_DATA_CONTRACT_VERSION,
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

#[derive(Clone, Debug, Serialize)]
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

#[cfg(test)]
mod tests {
    use nte_dps_tool::core::character_data::{CharacterDataProjection, CharacterDataRecord};

    use super::*;

    #[test]
    fn snapshot_uses_camel_case() {
        let snapshot = CharacterDataSnapshot::from_projection(CharacterDataProjection {
            records: vec![CharacterDataRecord {
                id: 1,
                name_zh: "角色".to_owned(),
                name_en: "Character".to_owned(),
                codename: "Code".to_owned(),
                attribute: "灵".to_owned(),
                verified: true,
                color: "#123ABC".to_owned(),
                avatar: "res/images/characters/player_001.png".to_owned(),
            }],
        });

        let value = serde_json::to_value(snapshot).expect("serialize character snapshot");

        assert_eq!(value["contractVersion"], CHARACTER_DATA_CONTRACT_VERSION);
        assert_eq!(value["records"][0]["nameZh"], "角色");
        assert_eq!(value["records"][0]["verified"], true);
        assert!(
            value["attributes"]
                .as_array()
                .is_some_and(|rows| rows.len() == 6)
        );
    }
}
