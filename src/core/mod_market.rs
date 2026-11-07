use std::collections::HashSet;
use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use ed25519_dalek::{Signature, VerifyingKey};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::storage::mod_scripts::validate_mod_id;
use std::{
    io::{Read, Write},
    path::Path,
};
pub const MAX_PLUGIN_BYTES: usize = 64 * 1024 * 1024;

pub const MOD_MARKET_SCHEMA_VERSION: u32 = 6;
pub const MOD_MARKET_CATALOG_URL: &str = "https://dps.o-na-ni.com/mods/v3/catalog.json";
pub const MOD_MARKET_PACKAGE_URL_PREFIX: &str = "https://dps.o-na-ni.com/mods/v3/packages/";
pub const MAX_MOD_MARKET_CATALOG_BYTES: usize = 128 * 1024;
pub const MAX_MOD_MARKET_ITEMS: usize = 64;
pub const MAX_MOD_MARKET_VERSION_BYTES: usize = 128;
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

/// The signed catalog selects a fixed destination, never an arbitrary local path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(feature = "tauri_frontend", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ComponentKind {
    #[default]
    Plugin,
    Host,
    Loader,
    Driver,
}
impl ComponentKind {
    fn extension(self) -> &'static str {
        match self {
            Self::Plugin | Self::Host => "dll",
            Self::Loader => "exe",
            Self::Driver => "sys",
        }
    }
    fn reserved_id(self) -> Option<&'static str> {
        match self {
            Self::Plugin => None,
            Self::Host => Some("nte-host"),
            Self::Loader => Some("nte-loader"),
            Self::Driver => Some("uetools-driver"),
        }
    }
    pub fn relative_path(self, id: &str) -> Result<std::path::PathBuf, ModMarketError> {
        validate_mod_id(id).map_err(|_| invalid_catalog("invalid component id"))?;
        if self.reserved_id().is_some_and(|expected| expected != id)
            || (self == Self::Plugin && ["nte-host", "nte-loader", "uetools-driver"].contains(&id))
        {
            return Err(invalid_catalog("component id does not match its kind"));
        }
        Ok(match self {
            Self::Plugin => std::path::PathBuf::from("game/plugins").join(format!("{id}.dll")),
            Self::Host => "game/d3d12.dll".into(),
            Self::Loader => "tools/NTE-Loader.exe".into(),
            Self::Driver => "driver/uetools.sys".into(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModMarketItem {
    pub component: ComponentKind,
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
    DeploymentConflict,
    ProxyIo,
    ProxyReceiptNotAFile,
    ProxyReceiptTooLarge,
    ProxyReceiptUnsupportedVersion,
    ProxyReceiptInvalid,
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
    #[serde(default)]
    component: Option<ComponentKind>,
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
    if !matches!(payload.schema, 5 | MOD_MARKET_SCHEMA_VERSION)
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
        let component = match (payload.schema, item.component) {
            (5, None | Some(ComponentKind::Plugin)) => ComponentKind::Plugin,
            (MOD_MARKET_SCHEMA_VERSION, Some(kind)) => kind,
            _ => {
                return Err(invalid_catalog(
                    "catalog component kind is missing or incompatible",
                ));
            }
        };
        component.relative_path(&item.id)?;
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
        let version = parse_catalog_version(&item.version)?;
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
        let expected_url = format!(
            "{MOD_MARKET_PACKAGE_URL_PREFIX}{}-{}.{}",
            item.id,
            version,
            component.extension()
        );
        if item.artifact.url != expected_url
            || item.artifact.size == 0
            || item.artifact.size > MAX_PLUGIN_BYTES as u64
        {
            return Err(invalid_catalog("catalog package metadata is invalid"));
        }
        let package_sha256 = decode_sha256(&item.artifact.sha256)?;
        mods.push(ModMarketItem {
            component,
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
) -> Result<Vec<u8>, ModMarketError> {
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
    validate_component_binary(bytes, item.component)?;
    Ok(bytes.to_vec())
}

pub fn mod_market_package_is_current(item: &ModMarketItem, bytes: &[u8]) -> bool {
    let actual: [u8; 32] = Sha256::digest(bytes).into();
    actual == item.package_sha256
}

pub fn validate_plugin_binary(bytes: &[u8]) -> Result<(), ModMarketError> {
    validate_component_binary(bytes, ComponentKind::Plugin)
}

fn validate_component_binary(bytes: &[u8], kind: ComponentKind) -> Result<(), ModMarketError> {
    let fail = || {
        ModMarketError::new(
            ModMarketErrorCode::InvalidPackage,
            "expected a compiled x64 PE of the declared component kind",
        )
    };
    if bytes.len() < 64 || bytes.len() > MAX_PLUGIN_BYTES || &bytes[..2] != b"MZ" {
        return Err(fail());
    }
    let pe = u32::from_le_bytes(bytes[60..64].try_into().map_err(|_| fail())?) as usize;
    let header = bytes
        .get(pe..pe.checked_add(94).ok_or_else(fail)?)
        .ok_or_else(fail)?;
    if &header[..4] != b"PE\0\0"
        || u16::from_le_bytes([header[4], header[5]]) != 0x8664
        || u16::from_le_bytes([header[24], header[25]]) != 0x20b
    {
        return Err(fail());
    }
    let dll = u16::from_le_bytes([header[22], header[23]]) & 0x2000 != 0;
    let subsystem = u16::from_le_bytes([header[92], header[93]]);
    let valid_kind = match kind {
        ComponentKind::Plugin | ComponentKind::Host => dll && matches!(subsystem, 2 | 3),
        ComponentKind::Loader => !dll && matches!(subsystem, 2 | 3),
        ComponentKind::Driver => !dll && subsystem == 1,
    };
    if !valid_kind {
        return Err(fail());
    }
    Ok(())
}

pub fn read_installed_component(
    directory: &Path,
    item: &ModMarketItem,
) -> Result<Option<Vec<u8>>, ModMarketError> {
    let path = directory.join(item.component.relative_path(&item.id)?);
    read_component_file(&path, item.component)
}

fn read_component_file(
    path: &Path,
    kind: ComponentKind,
) -> Result<Option<Vec<u8>>, ModMarketError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => {
            return Err(invalid_catalog("component is not a regular file"));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(invalid_catalog("component metadata failed")),
        _ => {}
    }
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(invalid_catalog("plugin read failed")),
    };
    let metadata = file
        .metadata()
        .map_err(|_| invalid_catalog("plugin metadata failed"))?;
    if !metadata.is_file() || metadata.len() > MAX_PLUGIN_BYTES as u64 {
        return Err(invalid_catalog("plugin size invalid"));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_PLUGIN_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid_catalog("plugin read failed"))?;
    validate_component_binary(&bytes, kind)?;
    Ok(Some(bytes))
}

pub fn install_component(
    directory: &Path,
    item: &ModMarketItem,
    bytes: &[u8],
) -> Result<(), ModMarketError> {
    let target = directory.join(item.component.relative_path(&item.id)?);
    let verified = verify_mod_market_package(item, bytes)?;
    crate::storage::io_util::atomic_write_file(&target, |writer| {
        writer.write_all(&verified).map_err(|e| e.to_string())
    })
    .map_err(|_| {
        ModMarketError::new(
            ModMarketErrorCode::InvalidPackage,
            "component replacement failed; previous file retained",
        )
    })
}

const PROXY_RECEIPT: &str = ".nte-proxy.json";
const MAX_PROXY_RECEIPT_BYTES: u64 = 16 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProxyReceipt {
    schema: u32,
    files: Vec<ProxyFile>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProxyFile {
    path: String,
    sha256: [u8; 32],
}

fn read_proxy_receipt(destination: &Path) -> Result<Option<ProxyReceipt>, ModMarketError> {
    let path = destination.join(PROXY_RECEIPT);
    let meta = match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        result => result.map_err(|_| proxy_io())?,
    };
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(ModMarketError::new(
            ModMarketErrorCode::ProxyReceiptNotAFile,
            "proxy receipt is not a regular file",
        ));
    }
    if meta.len() > MAX_PROXY_RECEIPT_BYTES {
        return Err(ModMarketError::new(
            ModMarketErrorCode::ProxyReceiptTooLarge,
            "proxy receipt exceeds size limit",
        ));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| proxy_io())?
        .take(MAX_PROXY_RECEIPT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| proxy_io())?;
    if bytes.len() as u64 > MAX_PROXY_RECEIPT_BYTES {
        return Err(ModMarketError::new(
            ModMarketErrorCode::ProxyReceiptTooLarge,
            "proxy receipt exceeds size limit",
        ));
    }
    let receipt: ProxyReceipt =
        serde_json::from_slice(&bytes).map_err(|_| invalid_proxy_receipt())?;
    let mut names = HashSet::new();
    if receipt.schema != 1 {
        return Err(ModMarketError::new(
            ModMarketErrorCode::ProxyReceiptUnsupportedVersion,
            "unsupported proxy receipt version",
        ));
    }
    if receipt.files.is_empty() || receipt.files.len() > MAX_MOD_MARKET_ITEMS + 1 {
        return Err(invalid_proxy_receipt());
    }
    for file in &receipt.files {
        let normalized = file.path.to_ascii_lowercase();
        let plugin = normalized
            .strip_prefix("plugins/")
            .and_then(|s| s.strip_suffix(".dll"));
        if normalized != "d3d12.dll" && !plugin.is_some_and(|id| validate_mod_id(id).is_ok()) {
            return Err(invalid_proxy_receipt());
        }
        if !names.insert(normalized) {
            return Err(invalid_proxy_receipt());
        }
    }
    if !names.contains("d3d12.dll") {
        return Err(invalid_proxy_receipt());
    }
    Ok(Some(receipt))
}

fn invalid_proxy_receipt() -> ModMarketError {
    ModMarketError::new(
        ModMarketErrorCode::ProxyReceiptInvalid,
        "invalid proxy receipt",
    )
}

fn proxy_io() -> ModMarketError {
    ModMarketError::new(
        ModMarketErrorCode::ProxyIo,
        "proxy file operation failed; close the game and retry",
    )
}

/// Remove only receipt-owned, unchanged binaries. Unknown files, configuration
/// and directories are retained. Missing files are allowed for an interrupted retry.
pub fn uninstall_proxy(runtime: &Path, destination: &Path) -> Result<(), ModMarketError> {
    let destination = std::fs::canonicalize(destination).map_err(|_| proxy_io())?;
    if !destination.join("HTGame.exe").is_file() {
        return Err(proxy_conflict());
    }
    let _lock = lock_proxy_directory(&destination)?;
    let receipt = read_proxy_receipt(&destination)?;
    let has_receipt = receipt.is_some();
    let receipt = if let Some(receipt) = receipt {
        receipt
    } else {
        // Pre-0.4.6 installations have no receipt: only a byte-identical managed
        // host is safe to remove. Preserve every plugin whose ownership is unknown.
        let Some(host) = read_component_file(&destination.join("d3d12.dll"), ComponentKind::Host)?
        else {
            return Ok(());
        };
        if read_component_file(&runtime.join("d3d12.dll"), ComponentKind::Host)?.as_deref()
            != Some(host.as_slice())
        {
            return Err(proxy_conflict());
        }
        ProxyReceipt {
            schema: 1,
            files: vec![ProxyFile {
                path: "d3d12.dll".into(),
                sha256: Sha256::digest(host).into(),
            }],
        }
    };
    if receipt
        .files
        .iter()
        .any(|f| f.path.to_ascii_lowercase().starts_with("plugins/"))
    {
        match std::fs::symlink_metadata(destination.join("plugins")) {
            Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(proxy_conflict()),
        }
    }
    // Complete all ownership checks before the first removal.
    let mut paths = Vec::new();
    for file in &receipt.files {
        let path = destination.join(&file.path);
        if let Some(bytes) =
            read_component_file(&path, ComponentKind::Plugin).map_err(|_| proxy_conflict())?
        {
            if <[u8; 32]>::from(Sha256::digest(&bytes)) != file.sha256 {
                return Err(proxy_conflict());
            }
            paths.push(path);
        }
    }
    // Disable auto-loading first, without ever recursively deleting the plugin directory.
    paths.sort_by_key(|p| {
        p.file_name()
            .is_none_or(|n| !n.to_string_lossy().eq_ignore_ascii_case("d3d12.dll"))
    });
    for path in paths {
        std::fs::remove_file(path).map_err(|_| proxy_io())?;
    }
    if has_receipt {
        std::fs::remove_file(destination.join(PROXY_RECEIPT)).map_err(|_| proxy_io())?;
    }
    Ok(())
}

/// Publish plugins before the host, then record ownership for updates and removal.
pub fn deploy_proxy(
    runtime: &Path,
    destination: &Path,
    force_overwrite: bool,
) -> Result<(), ModMarketError> {
    let destination = std::fs::canonicalize(destination).map_err(|_| proxy_io())?;
    if !destination.join("HTGame.exe").is_file() {
        return Err(invalid_catalog("game executable unavailable"));
    }
    let _lock = lock_proxy_directory(&destination)?;
    let files = proxy_package_files(runtime)?;
    let mut receipt = read_proxy_receipt(&destination)?.unwrap_or(ProxyReceipt {
        schema: 1,
        files: Vec::new(),
    });
    // The shared plugins directory also contains user settings and other plugins.
    match std::fs::symlink_metadata(destination.join("plugins")) {
        Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => return Err(proxy_io()),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(proxy_io()),
        _ => {}
    }
    let mut pending = Vec::new();
    let mut receipt_changed = false;
    for (relative, bytes) in files {
        let target = destination.join(&relative);
        let sha256: [u8; 32] = Sha256::digest(&bytes).into();
        let existing_hash = existing_proxy_hash(&target)?;
        let owned = receipt
            .files
            .iter_mut()
            .find(|file| file.path.eq_ignore_ascii_case(&relative));
        if existing_hash == Some(sha256) {
            // Legacy byte-identical hosts were already removable. Do not claim
            // pre-existing matching plugins that Toolkit never installed.
            if owned.is_none() && relative != "d3d12.dll" {
                continue;
            }
        } else {
            if existing_hash.is_some()
                && !force_overwrite
                && !owned
                    .as_ref()
                    .is_some_and(|file| Some(file.sha256) == existing_hash)
            {
                return Err(proxy_conflict());
            }
            pending.push(ProxyDeploymentFile {
                target,
                bytes,
                replace: existing_hash.is_some(),
            });
        }
        match owned {
            Some(file) if file.sha256 == sha256 => continue,
            Some(file) => file.sha256 = sha256,
            None => receipt.files.push(ProxyFile {
                path: relative,
                sha256,
            }),
        }
        receipt_changed = true;
    }
    if pending.is_empty() && !receipt_changed {
        return Ok(());
    }
    let receipt_bytes = serde_json::to_vec(&receipt).map_err(|_| proxy_io())?;
    if receipt.files.len() > MAX_MOD_MARKET_ITEMS + 1
        || receipt_bytes.len() as u64 > MAX_PROXY_RECEIPT_BYTES
    {
        return Err(ModMarketError::new(
            ModMarketErrorCode::ProxyReceiptTooLarge,
            "proxy receipt exceeds size limit",
        ));
    }
    publish_proxy_files(
        &destination,
        &pending,
        receipt_changed.then_some(receipt_bytes),
    )
}

struct ProxyDeploymentFile {
    target: std::path::PathBuf,
    bytes: Vec<u8>,
    replace: bool,
}

fn existing_proxy_hash(path: &Path) -> Result<Option<[u8; 32]>, ModMarketError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        _ => return Err(proxy_io()),
    }
    let mut file = std::fs::File::open(path).map_err(|_| proxy_io())?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 32 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|_| proxy_io())?;
        if n == 0 {
            return Ok(Some(hash.finalize().into()));
        }
        hash.update(&buffer[..n]);
    }
}

fn publish_proxy_files(
    destination: &Path,
    files: &[ProxyDeploymentFile],
    receipt: Option<Vec<u8>>,
) -> Result<(), ModMarketError> {
    use std::fs;
    // The deployment lock serializes writers. Never reuse a recovery directory
    // left by an interrupted deployment.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| proxy_io())?
        .as_nanos();
    let staging = destination.join(format!(".nte-stage-{}-{stamp}", std::process::id()));
    fs::create_dir(&staging).map_err(|_| proxy_io())?;
    let mut published = 0;
    let result = (|| {
        for (index, file) in files.iter().enumerate() {
            if file.replace {
                fs::copy(&file.target, staging.join(index.to_string())).map_err(|_| proxy_io())?;
            }
            write_proxy_file(&file.target, &file.bytes, file.replace)?;
            published += 1;
        }
        if let Some(bytes) = receipt {
            write_proxy_file(&destination.join(PROXY_RECEIPT), &bytes, true)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let mut rollback_failed = false;
        for (index, file) in files[..published].iter().enumerate().rev() {
            let restored = if file.replace {
                fs::rename(staging.join(index.to_string()), &file.target)
            } else {
                fs::remove_file(&file.target)
            };
            rollback_failed |= restored.is_err();
        }
        if rollback_failed {
            return Err(proxy_io()); // Keep remaining originals for recovery.
        }
    }
    fs::remove_dir_all(staging).map_err(|_| proxy_io())?;
    result
}

fn write_proxy_file(
    path: &Path,
    bytes: &[u8],
    replace_existing: bool,
) -> Result<(), ModMarketError> {
    crate::storage::io_util::atomic_publish_file(path, replace_existing, |writer| {
        writer.write_all(bytes).map_err(|error| error.to_string())
    })
    .map_err(|_| proxy_io())
}

fn lock_proxy_directory(destination: &Path) -> Result<std::fs::File, ModMarketError> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(destination.join(".nte-proxy.lock"))
        .map_err(|_| proxy_io())?;
    file.try_lock().map_err(|_| proxy_io())?;
    // Keep the lock file in place so concurrent processes always lock the same file.
    Ok(file)
}

const MAX_PROXY_DEPLOYMENT_BYTES: usize = 256 * 1024 * 1024;

fn proxy_package_files(runtime: &Path) -> Result<Vec<(String, Vec<u8>)>, ModMarketError> {
    use std::fs;
    let host = read_component_file(&runtime.join("d3d12.dll"), ComponentKind::Host)?
        .ok_or_else(|| invalid_catalog("managed host is missing"))?;
    let mut files = Vec::new();
    let mut total = host.len();
    let plugin_source = runtime.join("plugins");
    match fs::read_dir(&plugin_source) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.map_err(|_| invalid_catalog("plugin enumeration failed"))?;
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| invalid_catalog("invalid plugin filename"))?;
                if !name.to_ascii_lowercase().ends_with(".dll") {
                    continue;
                }
                if files.len() >= MAX_MOD_MARKET_ITEMS {
                    return Err(invalid_catalog("too many plugin files"));
                }
                validate_mod_id(name[..name.len() - 4].to_ascii_lowercase().as_str())
                    .map_err(|_| invalid_catalog("invalid plugin filename"))?;
                let bytes = read_component_file(&entry.path(), ComponentKind::Plugin)?
                    .ok_or_else(|| invalid_catalog("plugin disappeared"))?;
                total += bytes.len();
                if total > MAX_PROXY_DEPLOYMENT_BYTES {
                    return Err(invalid_catalog("proxy deployment exceeds byte budget"));
                }
                files.push((format!("plugins/{name}"), bytes));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(invalid_catalog("plugin enumeration failed")),
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files.push(("d3d12.dll".to_owned(), host));
    Ok(files)
}

fn proxy_conflict() -> ModMarketError {
    ModMarketError::new(
        ModMarketErrorCode::DeploymentConflict,
        "existing or changed game files were preserved",
    )
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

fn parse_catalog_version(value: &str) -> Result<Version, ModMarketError> {
    if value.is_empty() || value.len() > MAX_MOD_MARKET_VERSION_BYTES {
        return Err(invalid_catalog("catalog contains an invalid version"));
    }
    Version::parse(value).map_err(|_| invalid_catalog("catalog contains an invalid version"))
}

fn invalid_catalog(detail: &'static str) -> ModMarketError {
    ModMarketError::new(ModMarketErrorCode::InvalidCatalog, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component_fixture(kind: ComponentKind) -> (ModMarketItem, Vec<u8>) {
        let mut bytes = vec![0u8; 192];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&64u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        bytes[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
        bytes[86..88].copy_from_slice(
            &(if matches!(kind, ComponentKind::Host | ComponentKind::Plugin) {
                0x2000u16
            } else {
                2u16
            })
            .to_le_bytes(),
        );
        bytes[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
        bytes[156..158].copy_from_slice(
            &(if kind == ComponentKind::Driver {
                1u16
            } else {
                3u16
            })
            .to_le_bytes(),
        );
        let localized = ModMarketLocalizedText {
            name: "Fixture".into(),
            summary: "Fixture component".into(),
        };
        let id = kind.reserved_id().unwrap_or("nte_plugincombat").to_owned();
        let item = ModMarketItem {
            component: kind,
            id,
            bindings: vec!["toolkit".into()],
            localizations: ModMarketLocalizations {
                english: localized.clone(),
                simplified_chinese: localized.clone(),
                japanese: localized,
            },
            version: Version::new(1, 0, 0),
            author: "Fixture".into(),
            capabilities: vec![],
            package_url: String::new(),
            package_size: bytes.len() as u64,
            package_sha256: Sha256::digest(&bytes).into(),
        };
        (item, bytes)
    }
    fn temporary_directory(name: &str) -> std::path::PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("nte-market-{name}-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn every_component_has_a_fixed_target_and_pe_kind() {
        for kind in [
            ComponentKind::Host,
            ComponentKind::Plugin,
            ComponentKind::Loader,
            ComponentKind::Driver,
        ] {
            let (item, bytes) = component_fixture(kind);
            assert!(verify_mod_market_package(&item, &bytes).is_ok());
            let path = kind.relative_path(&item.id).unwrap();
            assert!(!path.is_absolute());
            assert!(kind.relative_path("../host").is_err());
            assert!(validate_component_binary(&bytes[..90], kind).is_err());
            if kind != ComponentKind::Plugin {
                assert!(kind.relative_path("arbitrary").is_err());
            }
            let mut wrong_arch = bytes.clone();
            wrong_arch[68] = 0;
            assert!(validate_component_binary(&wrong_arch, kind).is_err());
        }
        let (_, driver) = component_fixture(ComponentKind::Driver);
        assert!(validate_component_binary(&driver, ComponentKind::Loader).is_err());
        assert!(ComponentKind::Plugin.relative_path("nte-host").is_err());
    }
    #[test]
    fn component_install_rejects_corruption_and_retains_previous_file() {
        let root = temporary_directory("install");
        for kind in [
            ComponentKind::Host,
            ComponentKind::Plugin,
            ComponentKind::Loader,
            ComponentKind::Driver,
        ] {
            let (item, bytes) = component_fixture(kind);
            install_component(&root, &item, &bytes).unwrap();
            let mut corrupt = bytes.clone();
            corrupt[0] = 0;
            assert!(install_component(&root, &item, &corrupt).is_err());
            assert_eq!(read_installed_component(&root, &item).unwrap(), Some(bytes));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn proxy_deployment_is_idempotent_and_preserves_unknown_files() {
        let root = temporary_directory("proxy");
        let package = root.join("package");
        let game = root.join("game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("HTGame.exe"), []).unwrap();
        let (host_item, host) = component_fixture(ComponentKind::Host);
        let (plugin_item, plugin) = component_fixture(ComponentKind::Plugin);
        install_component(&package, &host_item, &host).unwrap();
        install_component(&package, &plugin_item, &plugin).unwrap();
        std::fs::write(game.join("d3d12.dll"), b"original").unwrap();
        assert!(deploy_proxy(&package.join("game"), &game, false).is_err());
        assert_eq!(std::fs::read(game.join("d3d12.dll")).unwrap(), b"original");
        assert!(!game.join("plugins").exists());
        std::fs::remove_file(game.join("d3d12.dll")).unwrap();
        deploy_proxy(&package.join("game"), &game, false).unwrap();
        deploy_proxy(&package.join("game"), &game, false).unwrap();
        assert_eq!(std::fs::read(game.join("d3d12.dll")).unwrap(), host);
        assert_eq!(
            std::fs::read(game.join("plugins/nte_plugincombat.dll")).unwrap(),
            plugin
        );
        std::fs::write(game.join("plugins/unknown.dll"), b"original plugin").unwrap();
        std::fs::write(game.join("plugins/settings.json"), b"user settings").unwrap();
        deploy_proxy(&package.join("game"), &game, false).unwrap();
        assert_eq!(
            std::fs::read(game.join("plugins/unknown.dll")).unwrap(),
            b"original plugin"
        );
        assert!(!std::fs::read_dir(&game).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".nte-stage")
        }));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn proxy_redeploy_updates_owned_files_adds_plugins_and_repairs_missing_files() {
        let root = temporary_directory("proxy-update");
        let package = root.join("package");
        let game = root.join("game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("HTGame.exe"), []).unwrap();
        let (host_item, host) = component_fixture(ComponentKind::Host);
        let (plugin_item, plugin) = component_fixture(ComponentKind::Plugin);
        install_component(&package, &host_item, &host).unwrap();
        deploy_proxy(&package.join("game"), &game, false).unwrap();
        install_component(&package, &plugin_item, &plugin).unwrap();
        deploy_proxy(&package.join("game"), &game, false).unwrap();
        let mut updated_host = host.clone();
        updated_host.push(1);
        let mut updated_plugin = plugin.clone();
        updated_plugin.push(2);
        std::fs::write(package.join("game/d3d12.dll"), &updated_host).unwrap();
        std::fs::write(
            package.join("game/plugins/nte_plugincombat.dll"),
            &updated_plugin,
        )
        .unwrap();
        deploy_proxy(&package.join("game"), &game, false).unwrap();
        assert_eq!(std::fs::read(game.join("d3d12.dll")).unwrap(), updated_host);
        assert_eq!(
            std::fs::read(game.join("plugins/nte_plugincombat.dll")).unwrap(),
            updated_plugin
        );
        for missing in ["d3d12.dll", "plugins/nte_plugincombat.dll"] {
            std::fs::remove_file(game.join(missing)).unwrap();
            deploy_proxy(&package.join("game"), &game, false).unwrap();
            assert!(game.join(missing).is_file());
        }
        uninstall_proxy(&package.join("game"), &game).unwrap();
        assert!(!game.join("d3d12.dll").exists());
        assert!(!game.join("plugins/nte_plugincombat.dll").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn proxy_overwrite_requires_confirmation_for_foreign_or_changed_binaries() {
        for recorded in [false, true] {
            let root = temporary_directory("proxy-force");
            let package = root.join("package");
            let game = root.join("game");
            std::fs::create_dir_all(game.join("plugins")).unwrap();
            std::fs::write(game.join("HTGame.exe"), []).unwrap();
            let (item, host) = component_fixture(ComponentKind::Host);
            install_component(&package, &item, &host).unwrap();
            let (item, plugin) = component_fixture(ComponentKind::Plugin);
            install_component(&package, &item, &plugin).unwrap();
            if recorded {
                deploy_proxy(&package.join("game"), &game, false).unwrap();
            }
            std::fs::write(game.join("d3d12.dll"), b"other host").unwrap();
            std::fs::write(game.join("plugins/nte_plugincombat.dll"), b"other plugin").unwrap();
            std::fs::write(game.join("plugins/config.json"), b"settings").unwrap();
            assert_eq!(
                deploy_proxy(&package.join("game"), &game, false)
                    .unwrap_err()
                    .code,
                ModMarketErrorCode::DeploymentConflict
            );
            assert_eq!(
                std::fs::read(game.join("d3d12.dll")).unwrap(),
                b"other host"
            );
            assert_eq!(
                std::fs::read(game.join("plugins/nte_plugincombat.dll")).unwrap(),
                b"other plugin"
            );
            deploy_proxy(&package.join("game"), &game, true).unwrap();
            assert_eq!(std::fs::read(game.join("d3d12.dll")).unwrap(), host);
            assert_eq!(
                std::fs::read(game.join("plugins/nte_plugincombat.dll")).unwrap(),
                plugin
            );
            uninstall_proxy(&package.join("game"), &game).unwrap();
            assert_eq!(
                std::fs::read(game.join("plugins/config.json")).unwrap(),
                b"settings"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(windows)]
    #[test]
    fn proxy_write_failures_restore_existing_files_and_allow_retry() {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
        for blocked in ["d3d12.dll", PROXY_RECEIPT, ".nte-proxy.lock"] {
            let root = temporary_directory("proxy-rollback");
            let package = root.join("package");
            let game = root.join("game");
            std::fs::create_dir_all(&game).unwrap();
            std::fs::write(game.join("HTGame.exe"), []).unwrap();
            let (item, host) = component_fixture(ComponentKind::Host);
            install_component(&package, &item, &host).unwrap();
            let (item, plugin) = component_fixture(ComponentKind::Plugin);
            install_component(&package, &item, &plugin).unwrap();
            deploy_proxy(&package.join("game"), &game, false).unwrap();
            let receipt = std::fs::read(game.join(PROXY_RECEIPT)).unwrap();
            let mut update = host.clone();
            update.push(1);
            std::fs::write(package.join("game/d3d12.dll"), &update).unwrap();
            std::fs::write(package.join("game/plugins/nte_plugincombat.dll"), &update).unwrap();
            let lock = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ)
                .open(game.join(blocked))
                .unwrap();
            assert_eq!(
                deploy_proxy(&package.join("game"), &game, true)
                    .unwrap_err()
                    .code,
                ModMarketErrorCode::ProxyIo
            );
            assert_eq!(std::fs::read(game.join("d3d12.dll")).unwrap(), host);
            assert_eq!(
                std::fs::read(game.join("plugins/nte_plugincombat.dll")).unwrap(),
                plugin
            );
            assert_eq!(std::fs::read(game.join(PROXY_RECEIPT)).unwrap(), receipt);
            drop(lock);
            deploy_proxy(&package.join("game"), &game, false).unwrap();
            assert_eq!(std::fs::read(game.join("d3d12.dll")).unwrap(), update);
            uninstall_proxy(&package.join("game"), &game).unwrap();
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn proxy_removal_preserves_foreign_files_and_survives_managed_updates() {
        let root = temporary_directory("proxy-remove");
        let package = root.join("package");
        let game = root.join("game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("HTGame.exe"), []).unwrap();
        let (item, host) = component_fixture(ComponentKind::Host);
        install_component(&package, &item, &host).unwrap();
        let (item, plugin) = component_fixture(ComponentKind::Plugin);
        install_component(&package, &item, &plugin).unwrap();
        deploy_proxy(&package.join("game"), &game, false).unwrap();
        std::fs::write(game.join("plugins/foreign.dll"), b"third party").unwrap();
        std::fs::write(game.join("plugins/settings.json"), b"user settings").unwrap();
        std::fs::write(package.join("game/d3d12.dll"), b"updated package").unwrap();
        // Changed owned binaries block removal before even the host is touched.
        std::fs::write(game.join("plugins/nte_plugincombat.dll"), b"modified").unwrap();
        assert!(uninstall_proxy(&package.join("game"), &game).is_err());
        assert_eq!(std::fs::read(game.join("d3d12.dll")).unwrap(), host);
        std::fs::write(game.join("plugins/nte_plugincombat.dll"), plugin).unwrap();
        uninstall_proxy(&package.join("game"), &game).unwrap();
        uninstall_proxy(&package.join("game"), &game).unwrap();
        assert!(!game.join("d3d12.dll").exists());
        assert!(!game.join("plugins/nte_plugincombat.dll").exists());
        assert!(!game.join(PROXY_RECEIPT).exists());
        assert_eq!(
            std::fs::read(game.join("plugins/foreign.dll")).unwrap(),
            b"third party"
        );
        assert_eq!(
            std::fs::read(game.join("plugins/settings.json")).unwrap(),
            b"user settings"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn proxy_removal_rejects_untrusted_receipts_and_preserves_legacy_plugins() {
        let root = temporary_directory("proxy-legacy");
        let runtime = root.join("runtime");
        let game = root.join("game");
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::create_dir_all(game.join("plugins")).unwrap();
        let (_, host) = component_fixture(ComponentKind::Host);
        std::fs::write(game.join("HTGame.exe"), []).unwrap();
        std::fs::write(runtime.join("d3d12.dll"), &host).unwrap();
        std::fs::write(game.join("d3d12.dll"), &host).unwrap();
        for path in [
            "../d3d12.dll",
            "plugins/../../d3d12.dll",
            "plugins/C:other.dll",
        ] {
            let receipt = ProxyReceipt {
                schema: 1,
                files: vec![ProxyFile {
                    path: path.into(),
                    sha256: [0; 32],
                }],
            };
            std::fs::write(
                game.join(PROXY_RECEIPT),
                serde_json::to_vec(&receipt).unwrap(),
            )
            .unwrap();
            assert!(uninstall_proxy(&runtime, &game).is_err());
            assert!(game.join("d3d12.dll").exists());
        }
        std::fs::write(
            game.join(PROXY_RECEIPT),
            vec![0; MAX_PROXY_RECEIPT_BYTES as usize + 1],
        )
        .unwrap();
        assert!(uninstall_proxy(&runtime, &game).is_err());
        assert_eq!(
            uninstall_proxy(&runtime, &game).unwrap_err().code,
            ModMarketErrorCode::ProxyReceiptTooLarge
        );
        std::fs::write(game.join(PROXY_RECEIPT), br#"{"schema":2,"files":[]}"#).unwrap();
        assert_eq!(
            uninstall_proxy(&runtime, &game).unwrap_err().code,
            ModMarketErrorCode::ProxyReceiptUnsupportedVersion
        );
        std::fs::write(game.join(PROXY_RECEIPT), b"malformed").unwrap();
        assert_eq!(
            uninstall_proxy(&runtime, &game).unwrap_err().code,
            ModMarketErrorCode::ProxyReceiptInvalid
        );
        std::fs::remove_file(game.join(PROXY_RECEIPT)).unwrap();
        std::fs::create_dir(game.join(PROXY_RECEIPT)).unwrap();
        assert_eq!(
            uninstall_proxy(&runtime, &game).unwrap_err().code,
            ModMarketErrorCode::ProxyReceiptNotAFile
        );
        std::fs::remove_dir(game.join(PROXY_RECEIPT)).unwrap();
        std::fs::write(game.join("plugins/old.dll"), b"old").unwrap();
        uninstall_proxy(&runtime, &game).unwrap();
        assert_eq!(std::fs::read(game.join("plugins/old.dll")).unwrap(), b"old");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[derive(Deserialize)]
    struct SemverConformanceVector {
        value: String,
        valid: bool,
    }

    #[test]
    fn catalog_semver_matches_shared_contract_conformance() {
        let vectors: Vec<SemverConformanceVector> =
            serde_json::from_str(include_str!("../../res/contract-semver-conformance.json"))
                .unwrap();
        for vector in vectors {
            assert_eq!(
                parse_catalog_version(&vector.value).is_ok(),
                vector.valid,
                "{}",
                vector.value
            );
        }
    }

    #[test]
    fn package_verification_checks_hash_and_local_source_policy() {
        let mut source = vec![0u8; 192];
        source[..2].copy_from_slice(b"MZ");
        source[60..64].copy_from_slice(&64u32.to_le_bytes());
        source[64..68].copy_from_slice(b"PE\0\0");
        source[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
        source[86..88].copy_from_slice(&0x2000u16.to_le_bytes());
        source[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
        source[156..158].copy_from_slice(&3u16.to_le_bytes());
        let item = ModMarketItem {
            component: ComponentKind::Plugin,
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
            package_url: format!("{MOD_MARKET_PACKAGE_URL_PREFIX}sample-1.0.0.dll"),
            package_size: source.len() as u64,
            package_sha256: Sha256::digest(source.as_slice()).into(),
        };

        assert_eq!(
            verify_mod_market_package(&item, source.as_slice()).unwrap(),
            source
        );
        let mut changed = source.as_slice().to_vec();
        changed[0] ^= 1;
        assert_eq!(
            verify_mod_market_package(&item, &changed).unwrap_err().code,
            ModMarketErrorCode::InvalidPackage
        );

        let source_text = b"#include <nte/mod.hpp>";
        let mismatched_item = ModMarketItem {
            package_size: source_text.len() as u64,
            package_sha256: Sha256::digest(source_text).into(),
            ..item
        };
        assert_eq!(
            verify_mod_market_package(&mismatched_item, source_text)
                .unwrap_err()
                .code,
            ModMarketErrorCode::InvalidPackage
        );
        println!("MARKET_BINARY_PASS: signed hash and x64 PE checks; scripts rejected");
    }

    #[test]
    fn package_urls_are_derived_from_id_and_semver() {
        assert_eq!(
            format!("{MOD_MARKET_PACKAGE_URL_PREFIX}sample-1.2.3.dll"),
            "https://dps.o-na-ni.com/mods/v3/packages/sample-1.2.3.dll"
        );
    }

    #[test]
    fn application_bindings_are_generic_identifiers() {
        assert!(validate_identifier("feature.dps-time-stop", 31).is_ok());
        assert!(validate_identifier("button.some-future-action", 31).is_ok());
        assert!(validate_identifier("Feature.Not-Stable", 31).is_err());
    }

    #[test]
    #[ignore = "requires NTE_MOD_MARKET_STAGED_DIRECTORY with a signed publication candidate"]
    fn staged_catalog_and_every_package_pass_local_verification() {
        let root = std::path::PathBuf::from(
            std::env::var_os("NTE_MOD_MARKET_STAGED_DIRECTORY").expect("staged directory"),
        );
        let path = root.join("catalog.json");
        assert!(path.metadata().unwrap().len() <= MAX_MOD_MARKET_CATALOG_BYTES as u64);
        let catalog = parse_mod_market_catalog(&std::fs::read(path).unwrap()).unwrap();
        for item in &catalog.mods {
            let name = item.package_url.rsplit('/').next().unwrap();
            let path = root.join("packages").join(name);
            assert_eq!(path.metadata().unwrap().len(), item.package_size);
            verify_mod_market_package(item, &std::fs::read(path).unwrap()).unwrap();
        }
        println!(
            "MARKET_STAGE_PASS: {} signed components",
            catalog.mods.len()
        );
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
