//! Frontend-neutral character-table loading, validation and atomic updates.

use std::{fmt, path::Path};

use serde_json::{Map, Value};

use crate::storage::{io_util::atomic_write_text, resource::read_resource_text};

pub const CHARACTER_ATTRIBUTES: [&str; 6] = ["灵", "咒", "光", "魂", "暗", "相"];
pub const CHARACTER_DATA_MAX_RECORDS: usize = 2_048;

const CHARACTER_NAME_MAX_LENGTH: usize = 128;
const CHARACTER_CODENAME_MAX_LENGTH: usize = 128;
const CHARACTER_AVATAR_MAX_LENGTH: usize = 512;

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CharacterDataRecordInput {
    pub original_id: Option<String>,
    pub id: String,
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
    IdAlreadyExists(u32),
    RecordMissing(u32),
    IdImmutable,
    NameRequired,
    InvalidColor,
    InvalidAttribute,
    FieldTooLong(&'static str),
    Serialize(String),
    Write(String),
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
            Self::IdAlreadyExists(id) => write!(formatter, "character ID {id} already exists"),
            Self::RecordMissing(id) => write!(formatter, "character ID {id} no longer exists"),
            Self::IdImmutable => write!(formatter, "an existing character ID cannot be changed"),
            Self::NameRequired => write!(formatter, "a Chinese or English name is required"),
            Self::InvalidColor => write!(formatter, "character color is not #RRGGBB"),
            Self::InvalidAttribute => write!(formatter, "character attribute is unsupported"),
            Self::FieldTooLong(field) => write!(formatter, "character field {field} is too long"),
            Self::Serialize(detail) => {
                write!(formatter, "character table serialization failed: {detail}")
            }
            Self::Write(detail) => write!(formatter, "character table write failed: {detail}"),
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

pub fn save_character_data_record(
    path: &Path,
    input: CharacterDataRecordInput,
) -> Result<CharacterDataProjection, CharacterDataError> {
    let text =
        read_resource_text(path).map_err(|error| CharacterDataError::Read(error.to_string()))?;
    let mut document = serde_json::from_str::<Value>(&text)
        .map_err(|error| CharacterDataError::Json(error.to_string()))?;
    apply_character_data_record(&mut document, input)?;
    let serialized = serde_json::to_string_pretty(&document)
        .map_err(|error| CharacterDataError::Serialize(error.to_string()))?;
    atomic_write_text(path, &format!("{serialized}\n")).map_err(CharacterDataError::Write)?;
    load_character_data(path)
}

pub fn apply_character_data_record(
    document: &mut Value,
    input: CharacterDataRecordInput,
) -> Result<CharacterDataRecord, CharacterDataError> {
    let input = validate_input(input)?;
    let characters = characters_mut(document)?;
    let id = input.id.to_string();

    match input.original_id {
        Some(original_id) => {
            if original_id != input.id {
                return Err(CharacterDataError::IdImmutable);
            }
            if !characters.contains_key(&id) {
                return Err(CharacterDataError::RecordMissing(input.id));
            }
        }
        None => {
            if characters.contains_key(&id) {
                return Err(CharacterDataError::IdAlreadyExists(input.id));
            }
            if characters.len() >= CHARACTER_DATA_MAX_RECORDS {
                return Err(CharacterDataError::TooManyRecords);
            }
        }
    }

    let row = characters
        .entry(id.clone())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| CharacterDataError::InvalidRecord(id.clone()))?;
    set_json_string(row, "name_zh", &input.name_zh);
    set_json_string(row, "name_en", &input.name_en);
    set_json_string(row, "codename", &input.codename);
    set_optional_json_string(row, "attribute", &input.attribute);
    row.insert("verified".to_owned(), Value::Bool(input.verified));
    set_optional_json_string(row, "color", &input.color);
    set_optional_json_string(row, "avatar", &input.avatar);
    Ok(CharacterDataRecord {
        id: input.id,
        name_zh: input.name_zh,
        name_en: input.name_en,
        codename: input.codename,
        attribute: input.attribute,
        verified: input.verified,
        color: input.color,
        avatar: input.avatar,
    })
}

struct ValidatedCharacterDataRecord {
    original_id: Option<u32>,
    id: u32,
    name_zh: String,
    name_en: String,
    codename: String,
    attribute: String,
    verified: bool,
    color: String,
    avatar: String,
}

fn validate_input(
    input: CharacterDataRecordInput,
) -> Result<ValidatedCharacterDataRecord, CharacterDataError> {
    let id = parse_id(&input.id)?;
    let original_id = input.original_id.as_deref().map(parse_id).transpose()?;
    let name_zh = trimmed_bounded(input.name_zh, "name_zh", CHARACTER_NAME_MAX_LENGTH)?;
    let name_en = trimmed_bounded(input.name_en, "name_en", CHARACTER_NAME_MAX_LENGTH)?;
    if name_zh.is_empty() && name_en.is_empty() {
        return Err(CharacterDataError::NameRequired);
    }
    let codename = trimmed_bounded(input.codename, "codename", CHARACTER_CODENAME_MAX_LENGTH)?;
    let attribute = input.attribute.trim().to_owned();
    if !attribute.is_empty() && !CHARACTER_ATTRIBUTES.contains(&attribute.as_str()) {
        return Err(CharacterDataError::InvalidAttribute);
    }
    let color = input.color.trim().to_owned();
    if !color.is_empty() && !is_hex_color(&color) {
        return Err(CharacterDataError::InvalidColor);
    }
    let avatar = trimmed_bounded(input.avatar, "avatar", CHARACTER_AVATAR_MAX_LENGTH)?;
    Ok(ValidatedCharacterDataRecord {
        original_id,
        id,
        name_zh,
        name_en,
        codename,
        attribute,
        verified: input.verified,
        color,
        avatar,
    })
}

fn parse_id(value: &str) -> Result<u32, CharacterDataError> {
    value
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or(CharacterDataError::InvalidId)
}

fn trimmed_bounded(
    value: String,
    field: &'static str,
    max_length: usize,
) -> Result<String, CharacterDataError> {
    let value = value.trim().to_owned();
    if value.chars().count() > max_length {
        return Err(CharacterDataError::FieldTooLong(field));
    }
    Ok(value)
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

fn characters_mut(document: &mut Value) -> Result<&mut Map<String, Value>, CharacterDataError> {
    document
        .get_mut("characters")
        .and_then(Value::as_object_mut)
        .ok_or(CharacterDataError::MissingCharacters)
}

fn json_string(row: &Map<String, Value>, key: &str) -> String {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn set_json_string(row: &mut Map<String, Value>, key: &str, value: &str) {
    row.insert(key.to_owned(), Value::String(value.to_owned()));
}

fn set_optional_json_string(row: &mut Map<String, Value>, key: &str, value: &str) {
    if value.is_empty() {
        row.remove(key);
    } else {
        set_json_string(row, key, value);
    }
}

fn is_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value.as_bytes()[1..]
            .iter()
            .all(|byte| byte.is_ascii_hexdigit())
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

    #[test]
    fn atomic_update_preserves_unknown_fields_and_reloads_projection() {
        let path = temporary_character_path("save");
        write_fixture(&path);
        let projection = save_character_data_record(
            &path,
            CharacterDataRecordInput {
                original_id: Some("20".to_owned()),
                id: "20".to_owned(),
                name_zh: "二十改".to_owned(),
                name_en: "Twenty".to_owned(),
                codename: "Twenty".to_owned(),
                attribute: "咒".to_owned(),
                verified: true,
                color: "#123ABC".to_owned(),
                avatar: "res/images/characters/player_020.png".to_owned(),
            },
        )
        .expect("save character data");

        assert_eq!(projection.records[1].name_zh, "二十改");
        let saved: Value = serde_json::from_str(&fs::read_to_string(&path).expect("saved text"))
            .expect("saved JSON");
        assert_eq!(saved["characters"]["20"]["future"], 42);
        fs::remove_dir_all(path.ancestors().nth(3).expect("fixture root")).expect("remove fixture");
    }

    #[test]
    fn validation_rejects_duplicate_id_and_invalid_fields() {
        let path = temporary_character_path("validation");
        write_fixture(&path);
        let base = CharacterDataRecordInput {
            id: "20".to_owned(),
            name_zh: "重复".to_owned(),
            ..Default::default()
        };

        assert_eq!(
            save_character_data_record(&path, base.clone()),
            Err(CharacterDataError::IdAlreadyExists(20))
        );
        assert_eq!(
            save_character_data_record(
                &path,
                CharacterDataRecordInput {
                    id: "21".to_owned(),
                    color: "red".to_owned(),
                    ..base.clone()
                }
            ),
            Err(CharacterDataError::InvalidColor)
        );
        assert_eq!(
            save_character_data_record(
                &path,
                CharacterDataRecordInput {
                    id: "21".to_owned(),
                    attribute: "water".to_owned(),
                    ..base
                }
            ),
            Err(CharacterDataError::InvalidAttribute)
        );
        fs::remove_dir_all(path.ancestors().nth(3).expect("fixture root")).expect("remove fixture");
    }
}
