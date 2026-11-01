use nte_dps_tool::core::encrypted_ini::{ENCRYPTED_INI_MAX_BYTES, EncryptedIniKey};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::encrypted_ini_service::EncryptedIniProjection;

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EncryptedIniSnapshot {
    pub generation: String,
    pub opened: bool,
    pub display_path: Option<String>,
    pub file_name: Option<String>,
    pub key: EncryptedIniKey,
    pub plaintext: String,
    pub encrypted_line_count: usize,
    pub max_bytes: u64,
}

impl From<EncryptedIniProjection> for EncryptedIniSnapshot {
    fn from(projection: EncryptedIniProjection) -> Self {
        Self {
            generation: projection.generation.to_string(),
            opened: projection.display_path.is_some(),
            display_path: projection.display_path,
            file_name: projection.file_name,
            key: projection.key,
            plaintext: projection.plaintext,
            encrypted_line_count: projection.encrypted_line_count,
            max_bytes: ENCRYPTED_INI_MAX_BYTES,
        }
    }
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenEncryptedIniResult {
    pub opened: bool,
    pub snapshot: EncryptedIniSnapshot,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SaveEncryptedIniRequest {
    pub expected_generation: String,
    pub key: EncryptedIniKey,
    pub plaintext: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveEncryptedIniResult {
    pub saved: bool,
    pub snapshot: EncryptedIniSnapshot,
}

pub(crate) fn parse_generation(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_uses_string_generation_and_camel_case_fields() {
        let value = serde_json::to_value(EncryptedIniSnapshot::from(EncryptedIniProjection {
            generation: u64::MAX,
            display_path: Some("Engine.ini".to_owned()),
            file_name: Some("Engine.ini".to_owned()),
            key: EncryptedIniKey::China,
            plaintext: "Value=1".to_owned(),
            encrypted_line_count: 3,
        }))
        .expect("serialize snapshot");
        assert_eq!(value["generation"], u64::MAX.to_string());
        assert_eq!(value["encryptedLineCount"], 3);
        assert!(value.get("encrypted_line_count").is_none());
    }

    #[test]
    fn save_request_rejects_unknown_fields() {
        let value = serde_json::json!({
            "expectedGeneration": "1",
            "key": "global",
            "plaintext": "Value=1",
            "extra": true
        });
        assert!(serde_json::from_value::<SaveEncryptedIniRequest>(value).is_err());
    }

    #[test]
    fn generation_parser_is_strict() {
        assert_eq!(parse_generation("42"), Some(42));
        assert_eq!(parse_generation("+42"), None);
    }
}
