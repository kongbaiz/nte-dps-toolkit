use std::collections::HashSet;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
use zip::ZipArchive;

use crate::core::update::{
    AvailableComponentUpdate, InstalledComponentVersions, UPDATE_HEALTH_MARKER_ENV,
    UPDATE_SCHEMA_VERSION, UpdateComponent, UpdateTransaction, safe_managed_release_path,
};
use crate::platform::{equipment_plugin, update_http, update_install};
use crate::storage::io_util::atomic_write_text;
use crate::storage::paths;

const MAX_ARCHIVE_ENTRIES: usize = 4_096;
const MAX_EXTRACTED_BYTES: u64 = 1024 * 1024 * 1024;
const UPDATE_ROOT_DIRECTORY: &str = ".update";
const COMPONENT_STATE_SCHEMA: u32 = 1;
const MAX_COMPONENT_STATE_BYTES: u64 = 64 * 1024;
const MAX_EQUIPMENT_PLUGIN_BYTES: u64 = 64 * 1024 * 1024;
const EQUIPMENT_PLUGIN_PATH: &str = "plugins/dwmapi.dll";
const EQUIPMENT_PLUGIN_BASELINE_VERSION_PATH: &str = "plugins/equipment-plugin.version";
const EQUIPMENT_PLUGIN_STATE_FILE: &str = "components.json";

#[derive(Clone, Debug)]
pub enum PreparedUpdate {
    App {
        version: Version,
        transaction_path: PathBuf,
        updater_path: PathBuf,
    },
    EquipmentPlugin {
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
            Self::EquipmentPlugin { .. } => UpdateComponent::EquipmentPlugin,
        }
    }

    pub fn version(&self) -> &Version {
        match self {
            Self::App { version, .. } | Self::EquipmentPlugin { version, .. } => version,
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
    MissingEquipmentPlugin,
    UnexpectedPluginArchiveContents,
    InvalidEquipmentPluginSize,
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
            Self::MissingEquipmentPlugin => {
                formatter.write_str("plugin update archive is missing plugins/dwmapi.dll")
            }
            Self::UnexpectedPluginArchiveContents => {
                formatter.write_str("plugin update archive contains unsupported files")
            }
            Self::InvalidEquipmentPluginSize => {
                formatter.write_str("equipment plugin file has an invalid size")
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
    Deployment(equipment_plugin::EquipmentPluginDeploymentError),
    Rollback(String),
}

impl fmt::Display for InstallPluginUpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongComponent => formatter.write_str("prepared update is not a plugin update"),
            Self::HashMismatch => {
                formatter.write_str("prepared equipment plugin hash does not match")
            }
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
    #[serde(default)]
    equipment_plugin: Option<ComponentStateEntry>,
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
    let plugin_path = install_dir.join(EQUIPMENT_PLUGIN_PATH);
    let equipment_plugin = if plugin_path.is_file() {
        match read_recorded_plugin_version(&install_dir, &plugin_path)? {
            Some(version) => Some(version),
            None => read_plugin_baseline_version(&install_dir)?.or(Some(app.clone())),
        }
    } else {
        None
    };
    Ok(InstalledComponentVersions {
        app,
        equipment_plugin,
    })
}

pub fn prepare_update(
    update: &AvailableComponentUpdate,
    progress: impl FnMut(u64, u64),
) -> Result<PreparedUpdate, PrepareUpdateError> {
    match update.component {
        UpdateComponent::App => prepare_app_update(update, progress),
        UpdateComponent::EquipmentPlugin => prepare_equipment_plugin_update(update, progress),
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

fn prepare_equipment_plugin_update(
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
    let plugin_relative = Path::new(EQUIPMENT_PLUGIN_PATH);
    if !files.iter().any(|path| path == plugin_relative) {
        return Err(PrepareUpdateError::MissingEquipmentPlugin);
    }
    if files.len() != 1 {
        return Err(PrepareUpdateError::UnexpectedPluginArchiveContents);
    }
    let plugin_path = staging_dir.join(plugin_relative);
    let plugin_size = plugin_path
        .metadata()
        .map_err(PrepareUpdateError::File)?
        .len();
    if plugin_size == 0 || plugin_size > MAX_EQUIPMENT_PLUGIN_BYTES {
        return Err(PrepareUpdateError::InvalidEquipmentPluginSize);
    }
    let plugin_sha256 = sha256_file(&plugin_path)?;
    let _ = fs::remove_file(package_path);
    Ok(PreparedUpdate::EquipmentPlugin {
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
    let PreparedUpdate::EquipmentPlugin {
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
    let target = install_dir.join(EQUIPMENT_PLUGIN_PATH);
    let backup_root = install_dir
        .join(UPDATE_ROOT_DIRECTORY)
        .join("backup")
        .join(transaction_id);
    let backup = backup_root.join(EQUIPMENT_PLUGIN_PATH);
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
    if let Err(error) = equipment_plugin::refresh_installed_equipment_plugins(&plugin) {
        rollback_plugin_source(&target, &backup, target_existed, transaction_id)?;
        restore_component_state(&install_dir, previous_state.as_deref())?;
        return Err(InstallPluginUpdateError::Deployment(error));
    }
    let _ = fs::remove_dir_all(backup_root);
    let _ = fs::remove_dir_all(staging_dir);
    Ok(())
}

fn read_plugin_baseline_version(
    install_dir: &Path,
) -> Result<Option<Version>, ComponentVersionError> {
    let path = install_dir.join(EQUIPMENT_PLUGIN_BASELINE_VERSION_PATH);
    let Some(text) = read_limited_text(&path, 128)? else {
        return Ok(None);
    };
    Version::parse(text.trim())
        .map(Some)
        .map_err(|error| ComponentVersionError::Invalid(error.to_string()))
}

fn read_recorded_plugin_version(
    install_dir: &Path,
    plugin_path: &Path,
) -> Result<Option<Version>, ComponentVersionError> {
    let state_path = component_state_path(install_dir);
    let state = match read_limited_text(&state_path, MAX_COMPONENT_STATE_BYTES) {
        Ok(Some(text)) => serde_json::from_str::<ComponentStateDocument>(&text)
            .map_err(|error| ComponentVersionError::Invalid(error.to_string()))?,
        Ok(None) => return Ok(None),
        Err(error) => return Err(error),
    };
    if state.schema != COMPONENT_STATE_SCHEMA {
        return Err(ComponentVersionError::Invalid(format!(
            "unsupported schema {}",
            state.schema
        )));
    }
    let Some(plugin) = state.equipment_plugin else {
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
    install_dir
        .join(UPDATE_ROOT_DIRECTORY)
        .join(EQUIPMENT_PLUGIN_STATE_FILE)
}

fn read_component_state_text(
    install_dir: &Path,
) -> Result<Option<String>, InstallPluginUpdateError> {
    read_limited_text(
        &component_state_path(install_dir),
        MAX_COMPONENT_STATE_BYTES,
    )
    .map_err(|error| InstallPluginUpdateError::State(error.to_string()))
}

fn write_plugin_component_state(
    install_dir: &Path,
    version: &Version,
    sha256: [u8; 32],
) -> Result<(), InstallPluginUpdateError> {
    let document = ComponentStateDocument {
        schema: COMPONENT_STATE_SCHEMA,
        equipment_plugin: Some(ComponentStateEntry {
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

pub fn mark_update_healthy_from_environment() -> io::Result<()> {
    let Some(marker) = std::env::var_os(UPDATE_HEALTH_MARKER_ENV).map(PathBuf::from) else {
        return Ok(());
    };
    let allowed_root = paths::software_dir()
        .canonicalize()?
        .join(UPDATE_ROOT_DIRECTORY)
        .join("health");
    if marker.parent() != Some(allowed_root.as_path()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "update health marker is outside the managed directory",
        ));
    }
    fs::create_dir_all(&allowed_root)?;
    let mut file = File::create(marker)?;
    file.write_all(b"healthy\n")?;
    file.sync_all()
}

pub fn cleanup_completed_update_staging() {
    let update_root = paths::software_dir().join(UPDATE_ROOT_DIRECTORY);
    let staging_root = update_root.join("staging");
    let transactions_root = update_root.join("transactions");
    let Ok(entries) = fs::read_dir(staging_root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(id) = path.file_name() else {
            continue;
        };
        let transaction = transactions_root.join(format!("{}.json", id.to_string_lossy()));
        if !transaction.is_file() {
            let _ = fs::remove_dir_all(path);
        }
    }
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
    fn recorded_plugin_version_requires_the_matching_plugin_hash() {
        let root = update_test_directory("plugin-version");
        let plugin_path = root.join(EQUIPMENT_PLUGIN_PATH);
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
        let version_path = root.join(EQUIPMENT_PLUGIN_BASELINE_VERSION_PATH);
        fs::create_dir_all(version_path.parent().unwrap()).unwrap();
        fs::write(&version_path, b"1.4.2\n").unwrap();

        assert_eq!(
            read_plugin_baseline_version(&root).unwrap(),
            Some(Version::parse("1.4.2").unwrap())
        );
        fs::remove_dir_all(root).unwrap();
    }
}
