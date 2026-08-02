use std::collections::HashSet;
use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use ed25519_dalek::{Signature, VerifyingKey};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::storage::mod_scripts::{MAX_MOD_SOURCE_BYTES, mod_source_bindings, validate_mod_id};

pub const MOD_MARKET_SCHEMA_VERSION: u32 = 4;
pub const MOD_MARKET_CATALOG_URL: &str = "https://dps.o-na-ni.com/mods/v1/catalog.json";
pub const MOD_MARKET_PACKAGE_URL_PREFIX: &str = "https://dps.o-na-ni.com/mods/v1/packages/";
pub const MAX_MOD_MARKET_CATALOG_BYTES: usize = 128 * 1024;
pub const MAX_MOD_MARKET_ITEMS: usize = 64;
const MOD_MARKET_KEY_ID: &str = "official-2026-07";
const MOD_MARKET_PUBLIC_KEY: [u8; 32] = [
    0xb6, 0x6d, 0x57, 0xe5, 0x5f, 0x79, 0x1a, 0x3e, 0x5d, 0xbd, 0x20, 0x40, 0x6f, 0x15, 0x0c, 0xd1,
    0x70, 0xc0, 0x6c, 0xe3, 0x04, 0xb5, 0x78, 0x7d, 0x44, 0x0f, 0xe7, 0xcf, 0x70, 0x2e, 0x7e, 0x08,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModMarketCatalog {
    pub published_at: String,
    pub mods: Vec<ModMarketItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModMarketItem {
    pub id: String,
    pub bindings: Vec<String>,
    pub localizations: ModMarketLocalizations,
    pub version: Version,
    pub author: String,
    pub capabilities: Vec<String>,
    pub package_url: String,
    pub package_size: u64,
    pub package_sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModMarketLocalizations {
    pub english: ModMarketLocalizedText,
    pub simplified_chinese: ModMarketLocalizedText,
    pub japanese: ModMarketLocalizedText,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModMarketLocalizedText {
    pub name: String,
    pub summary: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModMarketErrorCode {
    InvalidCatalog,
    InvalidSignature,
    InvalidPackage,
    ItemNotFound,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModMarketError {
    pub code: ModMarketErrorCode,
    pub detail: String,
}

impl ModMarketError {
    fn new(code: ModMarketErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for ModMarketError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for ModMarketError {}

#[derive(Deserialize)]
struct SignedCatalogEnvelope {
    payload: String,
    signatures: Vec<CatalogSignature>,
}

#[derive(Deserialize)]
struct CatalogSignature {
    key_id: String,
    signature: String,
}

#[derive(Deserialize)]
struct CatalogPayload {
    schema: u32,
    published_at: String,
    mods: Vec<CatalogItem>,
}

#[derive(Deserialize)]
struct CatalogItem {
    id: String,
    bindings: Vec<String>,
    localizations: CatalogLocalizations,
    version: String,
    author: String,
    #[serde(default)]
    capabilities: Vec<String>,
    artifact: CatalogArtifact,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogLocalizations {
    en: CatalogLocalizedText,
    #[serde(rename = "zh-CN")]
    zh_cn: CatalogLocalizedText,
    ja: CatalogLocalizedText,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogLocalizedText {
    name: String,
    summary: String,
}

#[derive(Deserialize)]
struct CatalogArtifact {
    url: String,
    size: u64,
    sha256: String,
}

pub fn parse_mod_market_catalog(bytes: &[u8]) -> Result<ModMarketCatalog, ModMarketError> {
    if bytes.is_empty() || bytes.len() > MAX_MOD_MARKET_CATALOG_BYTES {
        return Err(invalid_catalog("catalog size is outside the allowed range"));
    }
    let envelope: SignedCatalogEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| invalid_catalog("catalog envelope is invalid"))?;
    let signature = envelope
        .signatures
        .iter()
        .find(|signature| signature.key_id == MOD_MARKET_KEY_ID)
        .ok_or_else(|| {
            ModMarketError::new(
                ModMarketErrorCode::InvalidSignature,
                "catalog does not carry the required signature",
            )
        })?;
    verify_catalog_signature(envelope.payload.as_bytes(), &signature.signature)?;

    let payload: CatalogPayload = serde_json::from_str(&envelope.payload)
        .map_err(|_| invalid_catalog("catalog payload is invalid"))?;
    if payload.schema != MOD_MARKET_SCHEMA_VERSION
        || payload.published_at.is_empty()
        || payload.published_at.len() > 64
        || payload.published_at.chars().any(char::is_control)
        || payload.mods.is_empty()
        || payload.mods.len() > MAX_MOD_MARKET_ITEMS
    {
        return Err(invalid_catalog("catalog metadata is invalid"));
    }

    let mut ids = HashSet::with_capacity(payload.mods.len());
    let mut mods = Vec::with_capacity(payload.mods.len());
    for item in payload.mods {
        validate_mod_id(&item.id)
            .map_err(|_| invalid_catalog("catalog contains an invalid Mod ID"))?;
        if !ids.insert(item.id.clone()) {
            return Err(invalid_catalog("catalog contains duplicate Mod IDs"));
        }
        if item.bindings.is_empty() || item.bindings.len() > 16 {
            return Err(invalid_catalog("catalog binding list is invalid"));
        }
        let mut bindings = HashSet::with_capacity(item.bindings.len());
        for binding in &item.bindings {
            validate_identifier(binding, 31)?;
            if !bindings.insert(binding) {
                return Err(invalid_catalog("catalog contains duplicate bindings"));
            }
        }
        let version = Version::parse(&item.version)
            .map_err(|_| invalid_catalog("catalog contains an invalid version"))?;
        let localizations = ModMarketLocalizations {
            english: validate_localized_text(item.localizations.en)?,
            simplified_chinese: validate_localized_text(item.localizations.zh_cn)?,
            japanese: validate_localized_text(item.localizations.ja)?,
        };
        validate_text(&item.author, 64)?;
        if item.capabilities.len() > 16 {
            return Err(invalid_catalog("catalog capability list is too large"));
        }
        let mut capabilities = HashSet::with_capacity(item.capabilities.len());
        for capability in &item.capabilities {
            validate_identifier(capability, 64)?;
            if !capabilities.insert(capability) {
                return Err(invalid_catalog("catalog contains duplicate capabilities"));
            }
        }
        let expected_url = format!("{MOD_MARKET_PACKAGE_URL_PREFIX}{}-{}.nte", item.id, version);
        if item.artifact.url != expected_url
            || item.artifact.size == 0
            || item.artifact.size > MAX_MOD_SOURCE_BYTES as u64
        {
            return Err(invalid_catalog("catalog package metadata is invalid"));
        }
        let package_sha256 = decode_sha256(&item.artifact.sha256)?;
        mods.push(ModMarketItem {
            id: item.id,
            bindings: item.bindings,
            localizations,
            version,
            author: item.author,
            capabilities: item.capabilities,
            package_url: item.artifact.url,
            package_size: item.artifact.size,
            package_sha256,
        });
    }
    Ok(ModMarketCatalog {
        published_at: payload.published_at,
        mods,
    })
}

pub fn find_mod_market_item<'a>(
    catalog: &'a ModMarketCatalog,
    id: &str,
) -> Result<&'a ModMarketItem, ModMarketError> {
    validate_mod_id(id).map_err(|_| {
        ModMarketError::new(ModMarketErrorCode::ItemNotFound, "market Mod ID is invalid")
    })?;
    catalog
        .mods
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(|| {
            ModMarketError::new(
                ModMarketErrorCode::ItemNotFound,
                "market Mod was not present in the signed catalog",
            )
        })
}

pub fn verify_mod_market_package(
    item: &ModMarketItem,
    bytes: &[u8],
) -> Result<String, ModMarketError> {
    if bytes.len() as u64 != item.package_size {
        return Err(ModMarketError::new(
            ModMarketErrorCode::InvalidPackage,
            "market package size does not match the signed catalog",
        ));
    }
    let actual: [u8; 32] = Sha256::digest(bytes).into();
    if actual != item.package_sha256 {
        return Err(ModMarketError::new(
            ModMarketErrorCode::InvalidPackage,
            "market package checksum does not match the signed catalog",
        ));
    }
    let source = String::from_utf8(bytes.to_vec()).map_err(|_| {
        ModMarketError::new(
            ModMarketErrorCode::InvalidPackage,
            "market package is not UTF-8 source",
        )
    })?;
    let source_bindings = mod_source_bindings(&item.id, &source).map_err(|_| {
        ModMarketError::new(
            ModMarketErrorCode::InvalidPackage,
            "market package did not pass the local Mod validator",
        )
    })?;
    if source_bindings.len() != item.bindings.len()
        || source_bindings
            .iter()
            .any(|binding| !item.bindings.contains(binding))
    {
        return Err(ModMarketError::new(
            ModMarketErrorCode::InvalidPackage,
            "market package bindings do not match the signed catalog",
        ));
    }
    Ok(source)
}

pub fn mod_market_package_is_current(item: &ModMarketItem, source: &str) -> bool {
    let actual: [u8; 32] = Sha256::digest(source.as_bytes()).into();
    actual == item.package_sha256
}

fn verify_catalog_signature(payload: &[u8], encoded: &str) -> Result<(), ModMarketError> {
    let bytes = BASE64.decode(encoded).map_err(|_| {
        ModMarketError::new(
            ModMarketErrorCode::InvalidSignature,
            "catalog signature encoding is invalid",
        )
    })?;
    let signature = Signature::from_slice(&bytes).map_err(|_| {
        ModMarketError::new(
            ModMarketErrorCode::InvalidSignature,
            "catalog signature length is invalid",
        )
    })?;
    let key = VerifyingKey::from_bytes(&MOD_MARKET_PUBLIC_KEY).map_err(|_| {
        ModMarketError::new(
            ModMarketErrorCode::InvalidSignature,
            "compiled market public key is invalid",
        )
    })?;
    key.verify_strict(payload, &signature).map_err(|_| {
        ModMarketError::new(
            ModMarketErrorCode::InvalidSignature,
            "catalog signature verification failed",
        )
    })
}

fn decode_sha256(value: &str) -> Result<[u8; 32], ModMarketError> {
    let bytes = hex::decode(value).map_err(|_| invalid_catalog("package checksum is invalid"))?;
    bytes
        .try_into()
        .map_err(|_| invalid_catalog("package checksum length is invalid"))
}

fn validate_text(value: &str, maximum: usize) -> Result<(), ModMarketError> {
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(invalid_catalog("catalog text is invalid"));
    }
    Ok(())
}

fn validate_localized_text(
    value: CatalogLocalizedText,
) -> Result<ModMarketLocalizedText, ModMarketError> {
    validate_text(&value.name, 64)?;
    validate_text(&value.summary, 280)?;
    Ok(ModMarketLocalizedText {
        name: value.name,
        summary: value.summary,
    })
}

fn validate_identifier(value: &str, maximum: usize) -> Result<(), ModMarketError> {
    if value.is_empty()
        || value.len() > maximum
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
    {
        return Err(invalid_catalog("catalog identifier is invalid"));
    }
    Ok(())
}

fn invalid_catalog(detail: &'static str) -> ModMarketError {
    ModMarketError::new(ModMarketErrorCode::InvalidCatalog, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_verification_checks_hash_and_local_source_policy() {
        let source = crate::storage::mod_scripts::new_mod_script_template("sample")
            .unwrap()
            .replace(
                "NTE_MOD(\"sample\");",
                "NTE_MOD(\"sample\");\nNTE_BIND(\"feature.sample\");",
            );
        let item = ModMarketItem {
            id: "sample".to_owned(),
            bindings: vec!["feature.sample".to_owned()],
            localizations: ModMarketLocalizations {
                english: ModMarketLocalizedText {
                    name: "Sample".to_owned(),
                    summary: "Sample Mod".to_owned(),
                },
                simplified_chinese: ModMarketLocalizedText {
                    name: "示例".to_owned(),
                    summary: "示例 Mod".to_owned(),
                },
                japanese: ModMarketLocalizedText {
                    name: "サンプル".to_owned(),
                    summary: "サンプル Mod".to_owned(),
                },
            },
            version: Version::new(1, 0, 0),
            author: "NTE".to_owned(),
            capabilities: vec!["viewport.tick".to_owned()],
            package_url: format!("{MOD_MARKET_PACKAGE_URL_PREFIX}sample-1.0.0.nte"),
            package_size: source.len() as u64,
            package_sha256: Sha256::digest(source.as_bytes()).into(),
        };

        assert_eq!(
            verify_mod_market_package(&item, source.as_bytes()).unwrap(),
            source
        );
        let mut changed = source.as_bytes().to_vec();
        changed[0] ^= 1;
        assert_eq!(
            verify_mod_market_package(&item, &changed).unwrap_err().code,
            ModMarketErrorCode::InvalidPackage
        );

        let source_without_binding = source.replace("NTE_BIND(\"feature.sample\");\n", "");
        let mismatched_item = ModMarketItem {
            package_size: source_without_binding.len() as u64,
            package_sha256: Sha256::digest(source_without_binding.as_bytes()).into(),
            ..item
        };
        assert_eq!(
            verify_mod_market_package(&mismatched_item, source_without_binding.as_bytes())
                .unwrap_err()
                .code,
            ModMarketErrorCode::InvalidPackage
        );
    }

    #[test]
    fn package_urls_are_derived_from_id_and_semver() {
        assert_eq!(
            format!("{MOD_MARKET_PACKAGE_URL_PREFIX}sample-1.2.3.nte"),
            "https://dps.o-na-ni.com/mods/v1/packages/sample-1.2.3.nte"
        );
    }

    #[test]
    fn application_bindings_are_generic_identifiers() {
        assert!(validate_identifier("feature.dps-time-stop", 31).is_ok());
        assert!(validate_identifier("button.some-future-action", 31).is_ok());
        assert!(validate_identifier("Feature.Not-Stable", 31).is_err());
    }

    #[test]
    #[ignore = "requires the public Mod Market endpoint"]
    fn official_catalog_and_every_package_pass_local_verification() {
        let bytes = crate::platform::update_http::get_bytes(
            MOD_MARKET_CATALOG_URL,
            MAX_MOD_MARKET_CATALOG_BYTES,
        )
        .unwrap();
        let catalog = parse_mod_market_catalog(&bytes).unwrap();
        assert!(!catalog.mods.is_empty());
        for item in &catalog.mods {
            let package = crate::platform::update_http::get_bytes(
                &item.package_url,
                item.package_size as usize,
            )
            .unwrap();
            verify_mod_market_package(item, &package).unwrap();
        }
    }
}
