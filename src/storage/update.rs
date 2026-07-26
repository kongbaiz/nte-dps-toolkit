use std::collections::HashSet;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
use zip::ZipArchive;

use crate::core::update::{
    AvailableComponentUpdate, InstalledComponentVersions, UPDATE_HEALTH_MARKER_ENV,
    UPDATE_SCHEMA_VERSION, UpdateComponent, UpdateTransaction, safe_managed_release_path,
};
use crate::platform::{mods_plugin, update_http, update_install};
use crate::storage::io_util::atomic_write_text;
use crate::storage::paths;

const MAX_ARCHIVE_ENTRIES: usize = 4_096;
const MAX_EXTRACTED_BYTES: u64 = 1024 * 1024 * 1024;
const UPDATE_ROOT_DIRECTORY: &str = ".update";
const COMPONENT_STATE_SCHEMA: u32 = 1;
const MAX_COMPONENT_STATE_BYTES: u64 = 64 * 1024;
const MAX_MODS_PLUGIN_BYTES: u64 = 64 * 1024 * 1024;
const MODS_PLUGIN_PATH: &str = "plugins/dwmapi.dll";
const MODS_PLUGIN_BASELINE_VERSION_PATH: &str = "plugins/mods-plugin.version";
const MODS_PLUGIN_STATE_PATH: &str = "plugins/mods-plugin.state.json";
const LEGACY_MODS_PLUGIN_BASELINE_VERSION_PATH: &str = "plugins/equipment-plugin.version";
const LEGACY_MODS_PLUGIN_STATE_PATH: &str = "plugins/equipment-plugin.state.json";
const OLDER_COMPONENT_STATE_PATH: &str = ".update/components.json";
const COMPLETED_UPDATE_CLEANUP_DELAY: Duration = Duration::from_secs(1);
const COMPLETED_UPDATE_CLEANUP_RETRY_INTERVAL: Duration = Duration::from_millis(250);
const COMPLETED_UPDATE_CLEANUP_ATTEMPTS: usize = 40;

#[derive(Clone, Debug)]
pub enum PreparedUpdate {
    App {
        version: Version,
        transaction_path: PathBuf,
        updater_path: PathBuf,
    },
    ModsPlugin {
        version: Version,
        transaction_id: String,
        staging_dir: PathBuf,
        plugin_path: PathBuf,
        plugin_sha256: [u8; 32],
    },
}

impl PreparedUpdate {
    pub const fn component(&self) -> UpdateComponent {
        match self {
            Self::App { .. } => UpdateComponent::App,
            Self::ModsPlugin { .. } => UpdateComponent::ModsPlugin,
        }
    }

    pub fn version(&self) -> &Version {
        match self {
            Self::App { version, .. } | Self::ModsPlugin { version, .. } => version,
        }
    }
}

#[derive(Debug)]
pub enum PrepareUpdateError {
    Download(update_http::HttpError),
    HashMismatch,
    Archive(String),
    UnsafeArchivePath(String),
    UnsupportedArchivePath(String),
    TooManyArchiveEntries,
    ArchiveTooLarge,
    MissingApplication,
    MissingUpdater,
    MissingModsPlugin,
    UnexpectedPluginArchiveContents,
    InvalidModsPluginSize,
    File(io::Error),
    Transaction(String),
}

impl fmt::Display for PrepareUpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Download(error) => write!(formatter, "{error}"),
            Self::HashMismatch => formatter.write_str("downloaded update hash does not match"),
            Self::Archive(error) => write!(formatter, "update archive is invalid: {error}"),
            Self::UnsafeArchivePath(path) => {
                write!(formatter, "update archive contains an unsafe path: {path}")
            }
            Self::UnsupportedArchivePath(path) => {
                write!(
                    formatter,
                    "update archive contains an unsupported path: {path}"
                )
            }
            Self::TooManyArchiveEntries => {
                formatter.write_str("update archive contains too many entries")
            }
            Self::ArchiveTooLarge => {
                formatter.write_str("expanded update archive exceeds the size limit")
            }
            Self::MissingApplication => {
                formatter.write_str("update archive is missing nte-dps-tool.exe")
            }
            Self::MissingUpdater => {
                formatter.write_str("update archive is missing nte-updater.exe")
            }
            Self::MissingModsPlugin => {
                formatter.write_str("plugin update archive is missing plugins/dwmapi.dll")
            }
            Self::UnexpectedPluginArchiveContents => {
                formatter.write_str("plugin update archive contains unsupported files")
            }
            Self::InvalidModsPluginSize => {
                formatter.write_str("Mod loader file has an invalid size")
            }
            Self::File(error) => write!(formatter, "update file operation failed: {error}"),
            Self::Transaction(error) => write!(formatter, "update transaction is invalid: {error}"),
        }
    }
}

impl std::error::Error for PrepareUpdateError {}

#[derive(Debug)]
pub enum ComponentVersionError {
    File(io::Error),
    TooLarge,
    Invalid(String),
}

impl fmt::Display for ComponentVersionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(error) => write!(formatter, "component state file failed: {error}"),
            Self::TooLarge => formatter.write_str("component state file exceeds the size limit"),
            Self::Invalid(error) => write!(formatter, "component state file is invalid: {error}"),
        }
    }
}

impl std::error::Error for ComponentVersionError {}

#[derive(Debug)]
pub enum InstallPluginUpdateError {
    WrongComponent,
    HashMismatch,
    File(io::Error),
    State(String),
    Deployment(mods_plugin::ModsPluginDeploymentError),
    Rollback(String),
}

impl fmt::Display for InstallPluginUpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongComponent => formatter.write_str("prepared update is not a plugin update"),
            Self::HashMismatch => formatter.write_str("prepared Mod loader hash does not match"),
            Self::File(error) => write!(formatter, "plugin update file operation failed: {error}"),
            Self::State(error) => write!(formatter, "plugin update state failed: {error}"),
            Self::Deployment(error) => write!(formatter, "{error}"),
            Self::Rollback(error) => write!(formatter, "plugin update rollback failed: {error}"),
        }
    }
}

impl std::error::Error for InstallPluginUpdateError {}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ComponentStateDocument {
    schema: u32,
    #[serde(default, alias = "equipment_plugin")]
    mods_plugin: Option<ComponentStateEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ComponentStateEntry {
    version: String,
    sha256: String,
}

pub fn installed_component_versions(
    app: Version,
) -> Result<InstalledComponentVersions, ComponentVersionError> {
    let install_dir = paths::software_dir();
    let plugin_path = install_dir.join(MODS_PLUGIN_PATH);
    let mods_plugin = if plugin_path.is_file() {
        match read_recorded_plugin_version(&install_dir, &plugin_path)? {
            Some(version) => Some(version),
            None => read_plugin_baseline_version(&install_dir)?.or(Some(app.clone())),
        }
    } else {
        None
    };
    Ok(InstalledComponentVersions { app, mods_plugin })
}

pub fn prepare_update(
    update: &AvailableComponentUpdate,
    progress: impl FnMut(u64, u64),
) -> Result<PreparedUpdate, PrepareUpdateError> {
    match update.component {
        UpdateComponent::App => prepare_app_update(update, progress),
        UpdateComponent::ModsPlugin => prepare_mods_plugin_update(update, progress),
    }
}

fn prepare_app_update(
    update: &AvailableComponentUpdate,
    progress: impl FnMut(u64, u64),
) -> Result<PreparedUpdate, PrepareUpdateError> {
    let install_dir = paths::software_dir()
        .canonicalize()
        .map_err(PrepareUpdateError::File)?;
    let update_root = install_dir.join(UPDATE_ROOT_DIRECTORY);
    let hash_hex = hex::encode(update.artifact_sha256);
    let transaction_id = format!("app-{}-{}", update.version, &hash_hex[..12]);
    let package_path = update_root
        .join("downloads")
        .join(format!("{transaction_id}.zip.part"));
    update_http::download_file(
        &update.artifact_url,
        &package_path,
        update.artifact_size,
        progress,
    )
    .map_err(PrepareUpdateError::Download)?;
    if sha256_file(&package_path)? != update.artifact_sha256 {
        let _ = fs::remove_file(&package_path);
        return Err(PrepareUpdateError::HashMismatch);
    }

    let staging_dir = update_root.join("staging").join(&transaction_id);
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir).map_err(PrepareUpdateError::File)?;
    }
    fs::create_dir_all(&staging_dir).map_err(PrepareUpdateError::File)?;
    let files = extract_release_archive(&package_path, &staging_dir)?;
    if !files
        .iter()
        .any(|path| path == Path::new("nte-dps-tool.exe"))
    {
        return Err(PrepareUpdateError::MissingApplication);
    }
    if !files
        .iter()
        .any(|path| path == Path::new("nte-updater.exe"))
    {
        return Err(PrepareUpdateError::MissingUpdater);
    }

    let health_marker = update_root
        .join("health")
        .join(format!("{transaction_id}.ok"));
    if health_marker.exists() {
        fs::remove_file(&health_marker).map_err(PrepareUpdateError::File)?;
    }
    let transaction = UpdateTransaction {
        schema: UPDATE_SCHEMA_VERSION,
        id: transaction_id.clone(),
        parent_pid: std::process::id(),
        install_dir,
        staging_dir: staging_dir.clone(),
        health_marker,
        files,
    };
    let transaction_text = serde_json::to_string_pretty(&transaction)
        .map_err(|error| PrepareUpdateError::Transaction(error.to_string()))?;
    let transaction_path = update_root
        .join("transactions")
        .join(format!("{transaction_id}.json"));
    atomic_write_text(&transaction_path, &transaction_text)
        .map_err(PrepareUpdateError::Transaction)?;
    let _ = fs::remove_file(package_path);
    Ok(PreparedUpdate::App {
        version: update.version.clone(),
        transaction_path,
        updater_path: staging_dir.join("nte-updater.exe"),
    })
}

fn prepare_mods_plugin_update(
    update: &AvailableComponentUpdate,
    progress: impl FnMut(u64, u64),
) -> Result<PreparedUpdate, PrepareUpdateError> {
    let install_dir = paths::software_dir()
        .canonicalize()
        .map_err(PrepareUpdateError::File)?;
    let update_root = install_dir.join(UPDATE_ROOT_DIRECTORY);
    let hash_hex = hex::encode(update.artifact_sha256);
    let transaction_id = format!("plugin-{}-{}", update.version, &hash_hex[..12]);
    let package_path = update_root
        .join("downloads")
        .join(format!("{transaction_id}.zip.part"));
    update_http::download_file(
        &update.artifact_url,
        &package_path,
        update.artifact_size,
        progress,
    )
    .map_err(PrepareUpdateError::Download)?;
    if sha256_file(&package_path)? != update.artifact_sha256 {
        let _ = fs::remove_file(&package_path);
        return Err(PrepareUpdateError::HashMismatch);
    }

    let staging_dir = update_root.join("staging").join(&transaction_id);
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir).map_err(PrepareUpdateError::File)?;
    }
    fs::create_dir_all(&staging_dir).map_err(PrepareUpdateError::File)?;
    let files = extract_release_archive(&package_path, &staging_dir)?;
    let plugin_relative = Path::new(MODS_PLUGIN_PATH);
    if !files.iter().any(|path| path == plugin_relative) {
        return Err(PrepareUpdateError::MissingModsPlugin);
    }
    if files.len() != 1 {
        return Err(PrepareUpdateError::UnexpectedPluginArchiveContents);
    }
    let plugin_path = staging_dir.join(plugin_relative);
    let plugin_size = plugin_path
        .metadata()
        .map_err(PrepareUpdateError::File)?
        .len();
    if plugin_size == 0 || plugin_size > MAX_MODS_PLUGIN_BYTES {
        return Err(PrepareUpdateError::InvalidModsPluginSize);
    }
    let plugin_sha256 = sha256_file(&plugin_path)?;
    let _ = fs::remove_file(package_path);
    Ok(PreparedUpdate::ModsPlugin {
        version: update.version.clone(),
        transaction_id,
        staging_dir,
        plugin_path,
        plugin_sha256,
    })
}

pub fn launch_prepared_app_update(update: &PreparedUpdate) -> io::Result<Child> {
    let PreparedUpdate::App {
        transaction_path,
        updater_path,
        ..
    } = update
    else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "prepared update is not an application update",
        ));
    };
    let mut command = Command::new(updater_path);
    command
        .arg("--apply")
        .arg(transaction_path)
        .creation_flags(CREATE_NO_WINDOW);
    command.spawn()
}

pub fn install_prepared_plugin_update(
    update: &PreparedUpdate,
) -> Result<(), InstallPluginUpdateError> {
    let PreparedUpdate::ModsPlugin {
        version,
        transaction_id,
        staging_dir,
        plugin_path,
        plugin_sha256,
    } = update
    else {
        return Err(InstallPluginUpdateError::WrongComponent);
    };
    let install_dir = paths::software_dir()
        .canonicalize()
        .map_err(InstallPluginUpdateError::File)?;
    if sha256_file_io(plugin_path).map_err(InstallPluginUpdateError::File)? != *plugin_sha256 {
        return Err(InstallPluginUpdateError::HashMismatch);
    }
    let plugin = fs::read(plugin_path).map_err(InstallPluginUpdateError::File)?;
    let target = install_dir.join(MODS_PLUGIN_PATH);
    let backup_root = install_dir
        .join(UPDATE_ROOT_DIRECTORY)
        .join("backup")
        .join(transaction_id);
    let backup = backup_root.join(MODS_PLUGIN_PATH);
    let previous_state = read_component_state_text(&install_dir)?;
    let target_existed = target.is_file();
    if target_existed {
        copy_synced(&target, &backup).map_err(InstallPluginUpdateError::File)?;
    }

    write_plugin_component_state(&install_dir, version, *plugin_sha256)?;
    if let Err(error) =
        update_install::replace_file_from_source(plugin_path, &target, transaction_id)
    {
        restore_component_state(&install_dir, previous_state.as_deref())?;
        return Err(InstallPluginUpdateError::File(error));
    }
    if let Err(error) = mods_plugin::refresh_installed_mods_plugins(&plugin) {
        rollback_plugin_source(&target, &backup, target_existed, transaction_id)?;
        restore_component_state(&install_dir, previous_state.as_deref())?;
        return Err(InstallPluginUpdateError::Deployment(error));
    }
    let _ = fs::remove_dir_all(backup_root);
    let _ = fs::remove_dir_all(staging_dir);
    cleanup_completed_update_files(&install_dir);
    Ok(())
}

fn read_plugin_baseline_version(
    install_dir: &Path,
) -> Result<Option<Version>, ComponentVersionError> {
    let text = match read_limited_text(&install_dir.join(MODS_PLUGIN_BASELINE_VERSION_PATH), 128)? {
        Some(text) => text,
        None => {
            let legacy_path = install_dir.join(LEGACY_MODS_PLUGIN_BASELINE_VERSION_PATH);
            let Some(text) = read_limited_text(&legacy_path, 128)? else {
                return Ok(None);
            };
            text
        }
    };
    Version::parse(text.trim())
        .map(Some)
        .map_err(|error| ComponentVersionError::Invalid(error.to_string()))
}

fn read_recorded_plugin_version(
    install_dir: &Path,
    plugin_path: &Path,
) -> Result<Option<Version>, ComponentVersionError> {
    let Some(text) = read_component_state_text_raw(install_dir)? else {
        return Ok(None);
    };
    let state = serde_json::from_str::<ComponentStateDocument>(&text)
        .map_err(|error| ComponentVersionError::Invalid(error.to_string()))?;
    if state.schema != COMPONENT_STATE_SCHEMA {
        return Err(ComponentVersionError::Invalid(format!(
            "unsupported schema {}",
            state.schema
        )));
    }
    let Some(plugin) = state.mods_plugin else {
        return Ok(None);
    };
    let version = Version::parse(&plugin.version)
        .map_err(|error| ComponentVersionError::Invalid(error.to_string()))?;
    let expected_hash = decode_sha256(&plugin.sha256)?;
    let actual_hash = sha256_file_io(plugin_path).map_err(ComponentVersionError::File)?;
    Ok((actual_hash == expected_hash).then_some(version))
}

fn read_limited_text(
    path: &Path,
    maximum_bytes: u64,
) -> Result<Option<String>, ComponentVersionError> {
    let metadata = match path.metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ComponentVersionError::File(error)),
    };
    if metadata.len() > maximum_bytes {
        return Err(ComponentVersionError::TooLarge);
    }
    fs::read_to_string(path)
        .map(Some)
        .map_err(ComponentVersionError::File)
}

fn decode_sha256(value: &str) -> Result<[u8; 32], ComponentVersionError> {
    hex::decode(value.trim())
        .map_err(|error| ComponentVersionError::Invalid(error.to_string()))?
        .try_into()
        .map_err(|_| ComponentVersionError::Invalid("SHA-256 must be 32 bytes".to_owned()))
}

fn component_state_path(install_dir: &Path) -> PathBuf {
    install_dir.join(MODS_PLUGIN_STATE_PATH)
}

fn legacy_mods_plugin_state_path(install_dir: &Path) -> PathBuf {
    install_dir.join(LEGACY_MODS_PLUGIN_STATE_PATH)
}

fn older_component_state_path(install_dir: &Path) -> PathBuf {
    install_dir.join(OLDER_COMPONENT_STATE_PATH)
}

fn read_component_state_text_raw(
    install_dir: &Path,
) -> Result<Option<String>, ComponentVersionError> {
    for path in [
        component_state_path(install_dir),
        legacy_mods_plugin_state_path(install_dir),
        older_component_state_path(install_dir),
    ] {
        if let Some(text) = read_limited_text(&path, MAX_COMPONENT_STATE_BYTES)? {
            return Ok(Some(text));
        }
    }
    Ok(None)
}

fn read_component_state_text(
    install_dir: &Path,
) -> Result<Option<String>, InstallPluginUpdateError> {
    read_component_state_text_raw(install_dir)
        .map_err(|error| InstallPluginUpdateError::State(error.to_string()))
}

fn write_plugin_component_state(
    install_dir: &Path,
    version: &Version,
    sha256: [u8; 32],
) -> Result<(), InstallPluginUpdateError> {
    let document = ComponentStateDocument {
        schema: COMPONENT_STATE_SCHEMA,
        mods_plugin: Some(ComponentStateEntry {
            version: version.to_string(),
            sha256: hex::encode(sha256),
        }),
    };
    let text = serde_json::to_string_pretty(&document)
        .map_err(|error| InstallPluginUpdateError::State(error.to_string()))?;
    atomic_write_text(&component_state_path(install_dir), &text)
        .map_err(InstallPluginUpdateError::State)
}

fn restore_component_state(
    install_dir: &Path,
    previous: Option<&str>,
) -> Result<(), InstallPluginUpdateError> {
    let path = component_state_path(install_dir);
    if let Some(previous) = previous {
        atomic_write_text(&path, previous).map_err(InstallPluginUpdateError::State)
    } else {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(InstallPluginUpdateError::File(error)),
        }
    }
}

fn rollback_plugin_source(
    target: &Path,
    backup: &Path,
    target_existed: bool,
    transaction_id: &str,
) -> Result<(), InstallPluginUpdateError> {
    let result = if target_existed {
        update_install::replace_file_from_source(backup, target, transaction_id)
    } else {
        fs::remove_file(target)
    };
    result.map_err(|error| InstallPluginUpdateError::Rollback(error.to_string()))
}

fn copy_synced(source: &Path, destination: &Path) -> io::Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, destination)?;
    File::options().write(true).open(destination)?.sync_all()
}

pub fn mark_update_healthy_from_environment() -> io::Result<Option<PathBuf>> {
    let Some(marker) = std::env::var_os(UPDATE_HEALTH_MARKER_ENV).map(PathBuf::from) else {
        return Ok(None);
    };
    let allowed_root = paths::software_dir()
        .canonicalize()?
        .join(UPDATE_ROOT_DIRECTORY)
        .join("health");
    if marker.parent() != Some(allowed_root.as_path())
        || health_marker_transaction_id(&marker).is_none()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "update health marker is outside the managed directory",
        ));
    }
    fs::create_dir_all(&allowed_root)?;
    let mut file = File::create(&marker)?;
    file.write_all(b"healthy\n")?;
    file.sync_all()?;
    Ok(Some(marker))
}

pub fn cleanup_completed_update_staging() {
    cleanup_completed_update_files(&paths::software_dir());
}

pub fn cleanup_completed_app_update(marker: PathBuf) {
    let Some(transaction_id) = health_marker_transaction_id(&marker).map(str::to_owned) else {
        return;
    };
    let Some(update_root) = marker.parent().and_then(Path::parent).map(Path::to_owned) else {
        return;
    };
    std::thread::sleep(COMPLETED_UPDATE_CLEANUP_DELAY);
    for _ in 0..COMPLETED_UPDATE_CLEANUP_ATTEMPTS {
        cleanup_completed_transaction(&update_root, &transaction_id, &marker);
        if !update_root.exists() {
            return;
        }
        std::thread::sleep(COMPLETED_UPDATE_CLEANUP_RETRY_INTERVAL);
    }
}

fn cleanup_completed_update_root(update_root: &Path) {
    if !update_root.is_dir() || has_pending_update_transaction(update_root) {
        return;
    }
    let _ = fs::remove_dir_all(update_root);
}

fn cleanup_completed_update_files(install_dir: &Path) {
    if let Err(error) = migrate_legacy_plugin_metadata(install_dir) {
        eprintln!("Failed to migrate Mod loader update state: {error}");
        return;
    }
    cleanup_completed_update_root(&install_dir.join(UPDATE_ROOT_DIRECTORY));
}

fn migrate_legacy_plugin_metadata(install_dir: &Path) -> Result<(), String> {
    let state_path = component_state_path(install_dir);
    if !state_path.is_file() {
        for legacy_path in [
            legacy_mods_plugin_state_path(install_dir),
            older_component_state_path(install_dir),
        ] {
            let Some(text) = read_limited_text(&legacy_path, MAX_COMPONENT_STATE_BYTES)
                .map_err(|error| error.to_string())?
            else {
                continue;
            };
            let document: ComponentStateDocument =
                serde_json::from_str(&text).map_err(|error| error.to_string())?;
            if document.schema != COMPONENT_STATE_SCHEMA {
                return Err(format!("unsupported schema {}", document.schema));
            }
            let normalized =
                serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?;
            atomic_write_text(&state_path, &normalized)?;
            break;
        }
    }

    let legacy_state_path = legacy_mods_plugin_state_path(install_dir);
    if legacy_state_path.is_file() {
        fs::remove_file(&legacy_state_path).map_err(|error| error.to_string())?;
    }

    let baseline_path = install_dir.join(MODS_PLUGIN_BASELINE_VERSION_PATH);
    let legacy_baseline_path = install_dir.join(LEGACY_MODS_PLUGIN_BASELINE_VERSION_PATH);
    if !baseline_path.is_file()
        && let Some(text) =
            read_limited_text(&legacy_baseline_path, 128).map_err(|error| error.to_string())?
    {
        Version::parse(text.trim()).map_err(|error| error.to_string())?;
        atomic_write_text(&baseline_path, &text)?;
    }
    if legacy_baseline_path.is_file() {
        fs::remove_file(legacy_baseline_path).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn has_pending_update_transaction(update_root: &Path) -> bool {
    let Ok(entries) = fs::read_dir(update_root.join("transactions")) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "json")
    })
}

fn cleanup_completed_transaction(update_root: &Path, transaction_id: &str, marker: &Path) {
    let transactions_root = update_root.join("transactions");
    if transactions_root
        .join(format!("{transaction_id}.json"))
        .is_file()
    {
        return;
    }
    let _ = fs::remove_dir_all(update_root.join("staging").join(transaction_id));
    let _ = fs::remove_dir_all(update_root.join("backup").join(transaction_id));
    let _ = fs::remove_file(transactions_root.join(format!("{transaction_id}.log")));
    let _ = fs::remove_file(marker);
    for directory in ["downloads", "staging", "transactions", "health", "backup"] {
        let _ = fs::remove_dir(update_root.join(directory));
    }
    let _ = fs::remove_dir(update_root);
}

fn health_marker_transaction_id(marker: &Path) -> Option<&str> {
    let transaction_id = marker.file_name()?.to_str()?.strip_suffix(".ok")?;
    (!transaction_id.is_empty()
        && transaction_id.len() <= 96
        && transaction_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')))
    .then_some(transaction_id)
}

fn sha256_file(path: &Path) -> Result<[u8; 32], PrepareUpdateError> {
    sha256_file_io(path).map_err(PrepareUpdateError::File)
}

fn sha256_file_io(path: &Path) -> io::Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest.finalize().into())
}

fn extract_release_archive(
    package_path: &Path,
    staging_dir: &Path,
) -> Result<Vec<PathBuf>, PrepareUpdateError> {
    let package = File::open(package_path).map_err(PrepareUpdateError::File)?;
    let mut archive =
        ZipArchive::new(package).map_err(|error| PrepareUpdateError::Archive(error.to_string()))?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(PrepareUpdateError::TooManyArchiveEntries);
    }
    let mut extracted_bytes = 0_u64;
    let mut files = Vec::new();
    let mut paths = HashSet::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| PrepareUpdateError::Archive(error.to_string()))?;
        let relative = entry
            .enclosed_name()
            .ok_or_else(|| PrepareUpdateError::UnsafeArchivePath(entry.name().to_owned()))?
            .to_path_buf();
        if !safe_managed_release_path(&relative) {
            return Err(PrepareUpdateError::UnsupportedArchivePath(
                relative.display().to_string(),
            ));
        }
        if !paths.insert(release_path_key(&relative)) {
            return Err(PrepareUpdateError::Archive(format!(
                "duplicate path {}",
                relative.display()
            )));
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(PrepareUpdateError::UnsafeArchivePath(
                relative.display().to_string(),
            ));
        }
        extracted_bytes = extracted_bytes
            .checked_add(entry.size())
            .ok_or(PrepareUpdateError::ArchiveTooLarge)?;
        if extracted_bytes > MAX_EXTRACTED_BYTES {
            return Err(PrepareUpdateError::ArchiveTooLarge);
        }
        let target = staging_dir.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(PrepareUpdateError::File)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(PrepareUpdateError::File)?;
        }
        let mut output = File::create(&target).map_err(PrepareUpdateError::File)?;
        io::copy(&mut entry, &mut output).map_err(PrepareUpdateError::File)?;
        output.flush().map_err(PrepareUpdateError::File)?;
        output.sync_all().map_err(PrepareUpdateError::File)?;
        files.push(relative);
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn release_path_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update_test_directory(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("nte-update-{name}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path).expect("test directory should be created");
        path
    }

    #[test]
    fn release_allowlist_accepts_managed_files() {
        assert!(safe_managed_release_path(Path::new("nte-dps-tool.exe")));
        assert!(safe_managed_release_path(Path::new("plugins/dwmapi.dll")));
        assert!(safe_managed_release_path(Path::new("licenses/dep/LICENSE")));
    }

    #[test]
    fn release_allowlist_rejects_user_data_and_scripts() {
        assert!(!safe_managed_release_path(Path::new("config.json")));
        assert!(!safe_managed_release_path(Path::new(
            "history/session.json"
        )));
        assert!(!safe_managed_release_path(Path::new("install.ps1")));
    }

    #[test]
    fn relative_path_validation_rejects_parent_traversal() {
        assert!(!safe_managed_release_path(Path::new("../nte-dps-tool.exe")));
        assert!(safe_managed_release_path(Path::new("plugins/dwmapi.dll")));
    }

    #[test]
    fn archive_path_keys_follow_windows_case_insensitive_semantics() {
        assert_eq!(
            release_path_key(Path::new("plugins/dwmapi.dll")),
            release_path_key(Path::new("PLUGINS/DWMAPI.DLL"))
        );
    }

    #[test]
    fn completed_update_cleanup_removes_the_managed_root() {
        let root = update_test_directory("completed-cleanup");
        let update_root = root.join(UPDATE_ROOT_DIRECTORY);
        fs::create_dir_all(update_root.join("staging/app-0.3.6-hash")).unwrap();
        fs::create_dir_all(update_root.join("transactions")).unwrap();
        fs::create_dir_all(update_root.join("health")).unwrap();
        fs::write(
            update_root.join("transactions/app-0.3.6-hash.log"),
            b"update completed\n",
        )
        .unwrap();
        fs::write(update_root.join("health/app-0.3.6-hash.ok"), b"healthy\n").unwrap();

        cleanup_completed_update_root(&update_root);

        assert!(!update_root.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn completed_update_cleanup_preserves_a_pending_transaction() {
        let root = update_test_directory("pending-cleanup");
        let update_root = root.join(UPDATE_ROOT_DIRECTORY);
        let staging = update_root.join("staging/app-0.3.6-hash");
        let transaction = update_root.join("transactions/app-0.3.6-hash.json");
        fs::create_dir_all(&staging).unwrap();
        fs::create_dir_all(transaction.parent().unwrap()).unwrap();
        fs::write(&transaction, b"{}").unwrap();

        cleanup_completed_update_root(&update_root);

        assert!(staging.is_dir());
        assert!(transaction.is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn completed_update_cleanup_migrates_plugin_state_out_of_update_root() {
        let root = update_test_directory("state-migration");
        let plugin_path = root.join(MODS_PLUGIN_PATH);
        fs::create_dir_all(plugin_path.parent().unwrap()).unwrap();
        fs::write(&plugin_path, b"plugin-v2").unwrap();
        let plugin_hash = sha256_file_io(&plugin_path).unwrap();
        let legacy_state = older_component_state_path(&root);
        fs::create_dir_all(legacy_state.parent().unwrap()).unwrap();
        fs::write(
            &legacy_state,
            serde_json::to_vec_pretty(&ComponentStateDocument {
                schema: COMPONENT_STATE_SCHEMA,
                mods_plugin: Some(ComponentStateEntry {
                    version: "0.3.7".to_owned(),
                    sha256: hex::encode(plugin_hash),
                }),
            })
            .unwrap(),
        )
        .unwrap();

        cleanup_completed_update_files(&root);

        assert!(!root.join(UPDATE_ROOT_DIRECTORY).exists());
        assert!(component_state_path(&root).is_file());
        assert_eq!(
            read_recorded_plugin_version(&root, &plugin_path).unwrap(),
            Some(Version::parse("0.3.7").unwrap())
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retired_plugin_metadata_names_migrate_to_mods_plugin_names() {
        let root = update_test_directory("retired-plugin-metadata");
        let plugin_path = root.join(MODS_PLUGIN_PATH);
        fs::create_dir_all(plugin_path.parent().unwrap()).unwrap();
        fs::write(&plugin_path, b"plugin-v2").unwrap();
        let plugin_hash = sha256_file_io(&plugin_path).unwrap();
        let legacy_state = legacy_mods_plugin_state_path(&root);
        fs::write(
            &legacy_state,
            format!(
                "{{\"schema\":1,\"equipment_plugin\":{{\"version\":\"0.3.7\",\"sha256\":\"{}\"}}}}",
                hex::encode(plugin_hash)
            ),
        )
        .unwrap();
        let legacy_baseline = root.join(LEGACY_MODS_PLUGIN_BASELINE_VERSION_PATH);
        fs::write(&legacy_baseline, b"0.3.7\n").unwrap();

        cleanup_completed_update_files(&root);

        assert!(!legacy_state.exists());
        assert!(!legacy_baseline.exists());
        assert!(component_state_path(&root).is_file());
        assert!(root.join(MODS_PLUGIN_BASELINE_VERSION_PATH).is_file());
        assert_eq!(
            read_recorded_plugin_version(&root, &plugin_path).unwrap(),
            Some(Version::parse("0.3.7").unwrap())
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transaction_cleanup_does_not_touch_another_active_update() {
        let root = update_test_directory("transaction-cleanup");
        let update_root = root.join(UPDATE_ROOT_DIRECTORY);
        let completed_id = "app-0.3.6-hash";
        let marker = update_root
            .join("health")
            .join(format!("{completed_id}.ok"));
        fs::create_dir_all(update_root.join("staging").join(completed_id)).unwrap();
        fs::create_dir_all(update_root.join("backup").join(completed_id)).unwrap();
        fs::create_dir_all(marker.parent().unwrap()).unwrap();
        fs::create_dir_all(update_root.join("transactions")).unwrap();
        fs::write(
            update_root
                .join("transactions")
                .join(format!("{completed_id}.log")),
            b"update completed\n",
        )
        .unwrap();
        fs::write(&marker, b"healthy\n").unwrap();

        let active_staging = update_root.join("staging/plugin-0.3.7-hash");
        let active_download = update_root.join("downloads/plugin-0.3.7-hash.zip.part");
        fs::create_dir_all(&active_staging).unwrap();
        fs::create_dir_all(active_download.parent().unwrap()).unwrap();
        fs::write(&active_download, b"partial package").unwrap();

        cleanup_completed_transaction(&update_root, completed_id, &marker);

        assert!(!update_root.join("staging").join(completed_id).exists());
        assert!(!update_root.join("backup").join(completed_id).exists());
        assert!(!marker.exists());
        assert!(active_staging.is_dir());
        assert!(active_download.is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn health_marker_transaction_id_rejects_unsafe_names() {
        assert_eq!(
            health_marker_transaction_id(Path::new("health/app-0.3.6-0123456789ab.ok")),
            Some("app-0.3.6-0123456789ab")
        );
        assert_eq!(
            health_marker_transaction_id(Path::new("health/app update.ok")),
            None
        );
        assert_eq!(
            health_marker_transaction_id(Path::new("health/app-0.3.6.json")),
            None
        );
    }

    #[test]
    fn recorded_plugin_version_requires_the_matching_plugin_hash() {
        let root = update_test_directory("plugin-version");
        let plugin_path = root.join(MODS_PLUGIN_PATH);
        fs::create_dir_all(plugin_path.parent().unwrap()).unwrap();
        fs::write(&plugin_path, b"plugin-v2").unwrap();
        let hash = sha256_file_io(&plugin_path).unwrap();
        let version = Version::parse("0.3.7").unwrap();
        write_plugin_component_state(&root, &version, hash).unwrap();

        assert_eq!(
            read_recorded_plugin_version(&root, &plugin_path).unwrap(),
            Some(version)
        );

        fs::write(&plugin_path, b"plugin-from-app-package").unwrap();
        assert_eq!(
            read_recorded_plugin_version(&root, &plugin_path).unwrap(),
            None
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bundled_plugin_baseline_version_is_read_from_the_release_sidecar() {
        let root = update_test_directory("plugin-baseline");
        let version_path = root.join(MODS_PLUGIN_BASELINE_VERSION_PATH);
        fs::create_dir_all(version_path.parent().unwrap()).unwrap();
        fs::write(&version_path, b"1.4.2\n").unwrap();

        assert_eq!(
            read_plugin_baseline_version(&root).unwrap(),
            Some(Version::parse("1.4.2").unwrap())
        );
        fs::remove_dir_all(root).unwrap();
    }
}
