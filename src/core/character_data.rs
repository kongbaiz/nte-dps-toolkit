//! Frontend-neutral character-table loading.

use std::{fmt, path::Path};

use serde_json::{Map, Value};

use crate::storage::resource::read_resource_text;

pub const CHARACTER_ATTRIBUTES: [&str; 6] = ["灵", "咒", "光", "魂", "暗", "相"];
const CHARACTER_DATA_MAX_RECORDS: usize = 2_048;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CharacterDataProjection {
    pub records: Vec<CharacterDataRecord>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CharacterDataRecord {
    pub id: u32,
    pub name_zh: String,
    pub name_en: String,
    pub codename: String,
    pub attribute: String,
    pub verified: bool,
    pub color: String,
    pub avatar: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CharacterDataError {
    Read(String),
    Json(String),
    MissingCharacters,
    InvalidRecord(String),
    TooManyRecords,
    InvalidId,
}

impl fmt::Display for CharacterDataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(detail) => write!(formatter, "character table read failed: {detail}"),
            Self::Json(detail) => write!(formatter, "character table JSON is invalid: {detail}"),
            Self::MissingCharacters => {
                write!(formatter, "character table has no characters object")
            }
            Self::InvalidRecord(id) => write!(formatter, "character {id} is not an object"),
            Self::TooManyRecords => write!(formatter, "character table exceeds the record limit"),
            Self::InvalidId => write!(formatter, "character ID must be a positive integer"),
        }
    }
}

impl std::error::Error for CharacterDataError {}

pub fn load_character_data(path: &Path) -> Result<CharacterDataProjection, CharacterDataError> {
    let text =
        read_resource_text(path).map_err(|error| CharacterDataError::Read(error.to_string()))?;
    let document = serde_json::from_str::<Value>(&text)
        .map_err(|error| CharacterDataError::Json(error.to_string()))?;
    project_document(&document)
}

fn parse_id(value: &str) -> Result<u32, CharacterDataError> {
    value
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or(CharacterDataError::InvalidId)
}

fn project_document(document: &Value) -> Result<CharacterDataProjection, CharacterDataError> {
    let characters = document
        .get("characters")
        .and_then(Value::as_object)
        .ok_or(CharacterDataError::MissingCharacters)?;
    if characters.len() > CHARACTER_DATA_MAX_RECORDS {
        return Err(CharacterDataError::TooManyRecords);
    }
    let mut records = characters
        .iter()
        .map(|(id, value)| project_record(id, value))
        .collect::<Result<Vec<_>, _>>()?;
    records.sort_by_key(|record| record.id);
    Ok(CharacterDataProjection { records })
}

fn project_record(id: &str, value: &Value) -> Result<CharacterDataRecord, CharacterDataError> {
    let parsed_id = parse_id(id)?;
    let row = value
        .as_object()
        .ok_or_else(|| CharacterDataError::InvalidRecord(id.to_owned()))?;
    Ok(CharacterDataRecord {
        id: parsed_id,
        name_zh: json_string(row, "name_zh"),
        name_en: json_string(row, "name_en"),
        codename: json_string(row, "codename"),
        attribute: json_string(row, "attribute"),
        verified: row
            .get("verified")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        color: json_string(row, "color"),
        avatar: json_string(row, "avatar"),
    })
}

fn json_string(row: &Map<String, Value>, key: &str) -> String {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn temporary_character_path(tag: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "nte-character-data-{tag}-{}-{unique}",
                std::process::id()
            ))
            .join("res/data/characters/characters.json")
    }

    fn write_fixture(path: &Path) {
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture parent");
        fs::write(
            path,
            r#"{
  "characters": {
    "20": {"name_zh":"二十","name_en":"Twenty","future":42},
    "3": {"name_zh":"三","attribute":"灵","verified":true}
  }
}"#,
        )
        .expect("write fixture");
    }

    #[test]
    fn projection_sorts_numeric_ids_and_preserves_optional_fields() {
        let path = temporary_character_path("load");
        write_fixture(&path);

        let projection = load_character_data(&path).expect("load character data");

        assert_eq!(
            projection
                .records
                .iter()
                .map(|row| row.id)
                .collect::<Vec<_>>(),
            vec![3, 20]
        );
        assert_eq!(projection.records[0].attribute, "灵");
        assert!(projection.records[0].verified);
        fs::remove_dir_all(path.ancestors().nth(3).expect("fixture root")).expect("remove fixture");
    }
}
