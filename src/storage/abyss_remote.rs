//! Versioned remote delivery for the abyss value tables.
//!
//! Invariants:
//! - the tiny manifest is fetched on demand and is never trusted before schema,
//!   URL, size and digest validation;
//! - the content-addressed ZIP is downloaded only when the matching verified
//!   cache entry is absent;
//! - exactly four bounded JSON entries are accepted, and the engine validates
//!   their structure before the cached manifest becomes authoritative;
//! - a previously verified cache may be used when refresh fails, and that
//!   stale state is surfaced to the UI.

use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::sync::Mutex;

use chrono::DateTime;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use zip::ZipArchive;

use crate::engine::abyss_data::AbyssMonsterDataset;
use crate::platform::update_http;
use crate::storage::{io_util::atomic_write_text, paths};

pub const ABYSS_DATA_MANIFEST_URL: &str = "https://dps.o-na-ni.com/data/abyss/v1/manifest.json";
const ABYSS_DATA_ARTIFACT_URL_PREFIX: &str = "https://dps.o-na-ni.com/data/abyss/v1/abyss-data-";
const MANIFEST_SCHEMA: u32 = 1;
const MAX_MANIFEST_BYTES: usize = 16 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ENTRY_BYTES: u64 = 16 * 1024 * 1024;
const CACHE_DIRECTORY: &str = ".data-cache/abyss/v1";
const CACHE_MANIFEST: &str = "manifest.json";

const REQUIRED_ENTRIES: [&str; 4] = [
    "DT_MonsterStaticData_Abyss.json",
    "DT_MonsterPackData.json",
    "abyss_floor_monster_summary.json",
    "season_names_zh_cn.json",
];

// This is not a capture/reducer hot lock. It serializes the bounded remote
// refresh so two windows cannot race the same cache transaction.
static REMOTE_LOAD_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug)]
pub enum AbyssRemoteError {
    RuntimeUnavailable,
    ManifestDownload(update_http::HttpError),
    ManifestInvalid(String),
    Download(update_http::HttpError),
    Cache(io::Error),
    HashMismatch,
    Archive(String),
    Dataset(String),
    RefreshAndCacheFailed { refresh: String, cache: String },
}

impl fmt::Display for AbyssRemoteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RuntimeUnavailable => formatter.write_str("abyss data loader is unavailable"),
            Self::ManifestDownload(error) => {
                write!(formatter, "abyss manifest request failed: {error}")
            }
            Self::ManifestInvalid(detail) => {
                write!(formatter, "abyss manifest is invalid: {detail}")
            }
            Self::Download(error) => write!(formatter, "abyss package download failed: {error}"),
            Self::Cache(error) => write!(formatter, "abyss cache operation failed: {error}"),
            Self::HashMismatch => formatter.write_str("abyss package hash does not match manifest"),
            Self::Archive(detail) => write!(formatter, "abyss package is invalid: {detail}"),
            Self::Dataset(detail) => write!(formatter, "abyss dataset is invalid: {detail}"),
            Self::RefreshAndCacheFailed { refresh, cache } => write!(
                formatter,
                "abyss refresh failed ({refresh}) and no valid cache is available ({cache})"
            ),
        }
    }
}

impl std::error::Error for AbyssRemoteError {}

#[derive(Debug)]
pub struct LoadedAbyssDataset {
    pub dataset: AbyssMonsterDataset,
    pub data_version: String,
    pub updated_at: String,
    pub stale: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AbyssManifest {
    schema: u32,
    data_version: String,
    updated_at: String,
    artifact: AbyssArtifact,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AbyssArtifact {
    url: String,
    sha256: String,
    size: u64,
    uncompressed_size: u64,
}

pub fn load_latest_abyss_dataset() -> Result<LoadedAbyssDataset, AbyssRemoteError> {
    let _guard = REMOTE_LOAD_LOCK
        .lock()
        .map_err(|_| AbyssRemoteError::RuntimeUnavailable)?;
    let cache_root = paths::software_dir().join(CACHE_DIRECTORY);
    match refresh_dataset(&cache_root) {
        Ok(dataset) => Ok(dataset),
        Err(refresh) => match load_cached_dataset(&cache_root) {
            Ok(mut cached) => {
                cached.stale = true;
                Ok(cached)
            }
            Err(cache) => Err(AbyssRemoteError::RefreshAndCacheFailed {
                refresh: refresh.to_string(),
                cache: cache.to_string(),
            }),
        },
    }
}

fn refresh_dataset(cache_root: &Path) -> Result<LoadedAbyssDataset, AbyssRemoteError> {
    let bytes = update_http::get_bytes(ABYSS_DATA_MANIFEST_URL, MAX_MANIFEST_BYTES)
        .map_err(AbyssRemoteError::ManifestDownload)?;
    let manifest = parse_manifest(&bytes)?;
    fs::create_dir_all(cache_root).map_err(AbyssRemoteError::Cache)?;
    let archive_path = cache_root.join(archive_filename(&manifest));
    if !verified_archive_exists(&archive_path, &manifest)? {
        download_archive(&archive_path, &manifest)?;
    }
    let dataset = load_archive_dataset(&archive_path, &manifest)?;
    persist_cached_manifest(cache_root, &manifest)?;
    cleanup_old_archives(cache_root, &archive_path);
    Ok(loaded_dataset(dataset, manifest, false))
}

fn load_cached_dataset(cache_root: &Path) -> Result<LoadedAbyssDataset, AbyssRemoteError> {
    let manifest_path = cache_root.join(CACHE_MANIFEST);
    let metadata = manifest_path.metadata().map_err(AbyssRemoteError::Cache)?;
    if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES as u64 {
        return Err(AbyssRemoteError::ManifestInvalid(
            "cached manifest size is invalid".to_owned(),
        ));
    }
    let bytes = fs::read(&manifest_path).map_err(AbyssRemoteError::Cache)?;
    let manifest = parse_manifest(&bytes)?;
    let archive_path = cache_root.join(archive_filename(&manifest));
    if !verified_archive_exists(&archive_path, &manifest)? {
        return Err(AbyssRemoteError::HashMismatch);
    }
    let dataset = load_archive_dataset(&archive_path, &manifest)?;
    Ok(loaded_dataset(dataset, manifest, true))
}

fn loaded_dataset(
    dataset: AbyssMonsterDataset,
    manifest: AbyssManifest,
    stale: bool,
) -> LoadedAbyssDataset {
    LoadedAbyssDataset {
        dataset,
        data_version: manifest.data_version,
        updated_at: manifest.updated_at,
        stale,
    }
}

fn parse_manifest(bytes: &[u8]) -> Result<AbyssManifest, AbyssRemoteError> {
    let manifest: AbyssManifest = serde_json::from_slice(bytes)
        .map_err(|error| AbyssRemoteError::ManifestInvalid(error.to_string()))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn validate_manifest(manifest: &AbyssManifest) -> Result<(), AbyssRemoteError> {
    if manifest.schema != MANIFEST_SCHEMA {
        return Err(AbyssRemoteError::ManifestInvalid(
            "unsupported schema".to_owned(),
        ));
    }
    if manifest.data_version.is_empty()
        || manifest.data_version.len() > 128
        || !manifest.data_version.is_ascii()
    {
        return Err(AbyssRemoteError::ManifestInvalid(
            "dataVersion is invalid".to_owned(),
        ));
    }
    DateTime::parse_from_rfc3339(&manifest.updated_at)
        .map_err(|_| AbyssRemoteError::ManifestInvalid("updatedAt must be RFC 3339".to_owned()))?;
    if manifest.artifact.size == 0 || manifest.artifact.size > MAX_ARCHIVE_BYTES {
        return Err(AbyssRemoteError::ManifestInvalid(
            "compressed size is invalid".to_owned(),
        ));
    }
    if manifest.artifact.uncompressed_size == 0
        || manifest.artifact.uncompressed_size > MAX_EXPANDED_BYTES
    {
        return Err(AbyssRemoteError::ManifestInvalid(
            "uncompressed size is invalid".to_owned(),
        ));
    }
    let digest = decode_sha256(&manifest.artifact.sha256)?;
    let expected_url = format!(
        "{ABYSS_DATA_ARTIFACT_URL_PREFIX}{}.zip",
        hex::encode(digest)
    );
    if manifest.artifact.url != expected_url {
        return Err(AbyssRemoteError::ManifestInvalid(
            "artifact URL is outside the official content-addressed path".to_owned(),
        ));
    }
    Ok(())
}

fn decode_sha256(value: &str) -> Result<[u8; 32], AbyssRemoteError> {
    let mut digest = [0_u8; 32];
    hex::decode_to_slice(value, &mut digest)
        .map_err(|_| AbyssRemoteError::ManifestInvalid("sha256 is invalid".to_owned()))?;
    Ok(digest)
}

fn archive_filename(manifest: &AbyssManifest) -> String {
    format!("abyss-data-{}.zip", manifest.artifact.sha256)
}

fn verified_archive_exists(
    path: &Path,
    manifest: &AbyssManifest,
) -> Result<bool, AbyssRemoteError> {
    let metadata = match path.metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(AbyssRemoteError::Cache(error)),
    };
    if !metadata.is_file() || metadata.len() != manifest.artifact.size {
        return Ok(false);
    }
    let expected = decode_sha256(&manifest.artifact.sha256)?;
    Ok(sha256_file(path).map_err(AbyssRemoteError::Cache)? == expected)
}

fn download_archive(path: &Path, manifest: &AbyssManifest) -> Result<(), AbyssRemoteError> {
    let partial = path.with_extension("zip.part");
    update_http::download_file(
        &manifest.artifact.url,
        &partial,
        manifest.artifact.size,
        |_, _| {},
    )
    .map_err(AbyssRemoteError::Download)?;
    let expected = decode_sha256(&manifest.artifact.sha256)?;
    if sha256_file(&partial).map_err(AbyssRemoteError::Cache)? != expected {
        let _ = fs::remove_file(&partial);
        return Err(AbyssRemoteError::HashMismatch);
    }
    if path.exists() {
        fs::remove_file(path).map_err(AbyssRemoteError::Cache)?;
    }
    fs::rename(partial, path).map_err(AbyssRemoteError::Cache)
}

fn load_archive_dataset(
    path: &Path,
    manifest: &AbyssManifest,
) -> Result<AbyssMonsterDataset, AbyssRemoteError> {
    let file = File::open(path).map_err(AbyssRemoteError::Cache)?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| AbyssRemoteError::Archive(error.to_string()))?;
    if archive.len() != REQUIRED_ENTRIES.len() {
        return Err(AbyssRemoteError::Archive(
            "archive must contain exactly four tables".to_owned(),
        ));
    }
    let mut entries = HashMap::<String, Vec<u8>>::new();
    let mut expanded = 0_u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| AbyssRemoteError::Archive(error.to_string()))?;
        let name = entry.name().to_owned();
        if !REQUIRED_ENTRIES.contains(&name.as_str()) || entries.contains_key(&name) {
            return Err(AbyssRemoteError::Archive(format!(
                "unsupported or duplicate entry {name}"
            )));
        }
        let entry_size = entry.size();
        if !entry.is_file() || entry_size == 0 || entry_size > MAX_ENTRY_BYTES {
            return Err(AbyssRemoteError::Archive(format!(
                "entry {name} has an invalid size"
            )));
        }
        expanded = expanded
            .checked_add(entry_size)
            .ok_or_else(|| AbyssRemoteError::Archive("expanded size overflow".to_owned()))?;
        if expanded > MAX_EXPANDED_BYTES {
            return Err(AbyssRemoteError::Archive(
                "expanded data exceeds the size budget".to_owned(),
            ));
        }
        let capacity = usize::try_from(entry_size)
            .map_err(|_| AbyssRemoteError::Archive("entry is too large".to_owned()))?;
        let mut bytes = Vec::with_capacity(capacity);
        entry
            .take(entry_size.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| AbyssRemoteError::Archive(error.to_string()))?;
        if bytes.len() != capacity {
            return Err(AbyssRemoteError::Archive(format!(
                "entry {name} size changed while reading"
            )));
        }
        entries.insert(name, bytes);
    }
    if expanded != manifest.artifact.uncompressed_size {
        return Err(AbyssRemoteError::Archive(
            "expanded size does not match manifest".to_owned(),
        ));
    }
    let required = |name: &str| {
        entries
            .get(name)
            .map(Vec::as_slice)
            .ok_or_else(|| AbyssRemoteError::Archive(format!("missing entry {name}")))
    };
    AbyssMonsterDataset::from_remote_tables(
        required(REQUIRED_ENTRIES[0])?,
        required(REQUIRED_ENTRIES[1])?,
        required(REQUIRED_ENTRIES[2])?,
        required(REQUIRED_ENTRIES[3])?,
    )
    .map_err(|error| AbyssRemoteError::Dataset(error.to_string()))
}

fn persist_cached_manifest(
    cache_root: &Path,
    manifest: &AbyssManifest,
) -> Result<(), AbyssRemoteError> {
    let text = serde_json::to_string(manifest)
        .map_err(|error| AbyssRemoteError::ManifestInvalid(error.to_string()))?;
    atomic_write_text(&cache_root.join(CACHE_MANIFEST), &text)
        .map_err(|error| AbyssRemoteError::Cache(io::Error::other(error)))
}

fn cleanup_old_archives(cache_root: &Path, keep: &Path) {
    let Ok(entries) = fs::read_dir(cache_root) else {
        return;
    };
    for entry in entries.flatten().take(64) {
        let path = entry.path();
        if path != keep
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("abyss-data-") && name.ends_with(".zip"))
        {
            let _ = fs::remove_file(path);
        }
    }
}

fn sha256_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;

    fn manifest_for(archive: &[u8], uncompressed_size: u64) -> AbyssManifest {
        let sha256 = hex::encode(Sha256::digest(archive));
        AbyssManifest {
            schema: 1,
            data_version: "fixture-v1".to_owned(),
            updated_at: "2026-08-20T00:00:00Z".to_owned(),
            artifact: AbyssArtifact {
                url: format!("{ABYSS_DATA_ARTIFACT_URL_PREFIX}{sha256}.zip"),
                sha256,
                size: archive.len() as u64,
                uncompressed_size,
            },
        }
    }

    fn valid_tables() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            (
                REQUIRED_ENTRIES[0],
                r#"[{"Rows":{"mon_01":{"Comment":"测试怪","Tags":["mon_01"]}}}]"#
                    .as_bytes()
                    .to_vec(),
            ),
            (
                REQUIRED_ENTRIES[1],
                br#"[{"Rows":{"Abyss_1_1_mon_01":{"HPMaxBase":1234.0}}}]"#.to_vec(),
            ),
            (
                REQUIRED_ENTRIES[2],
                r#"{"rows":[{"abyss":"Abyss_1","level_id":1,"route":"FirstHalf","wave":1,"monster_pool_id":"pool","monsters":[{"attribute_id":"Abyss_1_1_mon_01","name":"测试怪","count":1}]}]}"#
                    .as_bytes()
                    .to_vec(),
            ),
            (
                REQUIRED_ENTRIES[3],
                r#"{"1":"测试赛季"}"#.as_bytes().to_vec(),
            ),
        ]
    }

    fn zip_tables(tables: &[(&str, Vec<u8>)]) -> (Vec<u8>, u64) {
        let mut output = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut output);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o644);
        let mut expanded = 0_u64;
        for (name, bytes) in tables {
            zip.start_file(*name, options).expect("start fixture entry");
            zip.write_all(bytes).expect("write fixture entry");
            expanded += bytes.len() as u64;
        }
        zip.finish().expect("finish fixture archive");
        (output.into_inner(), expanded)
    }

    #[test]
    fn rejects_manifest_that_redirects_artifact_official_path() {
        let mut manifest = manifest_for(b"zip", 10);
        manifest.artifact.url = "https://example.invalid/abyss.zip".to_owned();
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn loads_exact_bounded_archive_and_builds_dataset() {
        let (archive, expanded) = zip_tables(&valid_tables());
        let manifest = manifest_for(&archive, expanded);
        let path = std::env::temp_dir().join(format!(
            "nte-abyss-remote-valid-{}-{}.zip",
            std::process::id(),
            expanded
        ));
        fs::write(&path, &archive).expect("write fixture archive");
        let dataset = load_archive_dataset(&path, &manifest).expect("load remote dataset");
        fs::remove_file(path).expect("remove fixture archive");
        let floor = dataset.floor(1, 1).expect("fixture floor");
        assert_eq!(floor.monster_count(), 1);
        assert_eq!(floor.monsters[0].stats.hp_max_base, 1234.0);
    }

    #[test]
    fn rejects_archive_with_unexpected_entry_before_dataset_parse() {
        let mut tables = valid_tables();
        tables.pop();
        tables.push(("unexpected.json", b"{}".to_vec()));
        let (archive, expanded) = zip_tables(&tables);
        let manifest = manifest_for(&archive, expanded);
        let path = std::env::temp_dir().join(format!(
            "nte-abyss-remote-invalid-{}-{}.zip",
            std::process::id(),
            expanded
        ));
        fs::write(&path, archive).expect("write fixture archive");
        let error = load_archive_dataset(&path, &manifest).expect_err("reject archive");
        fs::remove_file(path).expect("remove fixture archive");
        assert!(matches!(error, AbyssRemoteError::Archive(_)));
    }

    #[test]
    fn verified_cache_restores_dataset_as_stale() {
        let (archive, expanded) = zip_tables(&valid_tables());
        let manifest = manifest_for(&archive, expanded);
        let root = std::env::temp_dir().join(format!(
            "nte-abyss-remote-cache-{}-{}",
            std::process::id(),
            expanded
        ));
        fs::create_dir_all(&root).expect("create cache fixture");
        fs::write(root.join(archive_filename(&manifest)), archive).expect("write cached archive");
        fs::write(
            root.join(CACHE_MANIFEST),
            serde_json::to_vec(&manifest).expect("serialize cached manifest"),
        )
        .expect("write cached manifest");

        let loaded = load_cached_dataset(&root).expect("load verified cache");

        assert!(loaded.stale);
        assert_eq!(loaded.dataset.seasons.len(), 1);
        fs::remove_dir_all(root).expect("remove cache fixture");
    }

    #[test]
    #[ignore = "requires the live official HTTPS endpoint"]
    fn loads_live_official_package_and_reports_current_metadata() {
        let loaded = load_latest_abyss_dataset().expect("load official abyss data");
        assert_eq!(loaded.dataset.seasons.len(), 10);
        assert!(!loaded.data_version.is_empty());
        assert!(!loaded.stale);
    }
}
