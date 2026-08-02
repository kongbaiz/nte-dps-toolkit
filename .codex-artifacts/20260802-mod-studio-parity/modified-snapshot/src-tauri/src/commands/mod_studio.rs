use std::path::{Path, PathBuf};

use nte_dps_tool::{
    platform::mods_plugin::{
        ModsPluginDeploymentError, ModsPluginGameRegion, inspect_plugin_deployment_with_manual,
        install_mods_plugin_with_manual, remove_mods_plugin_with_manual,
    },
    storage::{i18n, resource::read_mods_plugin},
};
use tauri::{State, WebviewWindow};

use crate::{
    contract::{
        CommandError,
        mod_studio::{
            ModStudioDeploymentSnapshot, ModStudioDirectorySelectionSnapshot,
            ModStudioDocumentSnapshot, ModStudioSdkSchemaSnapshot, ModStudioWorkspaceSnapshot,
        },
    },
    state::AppState,
    windows::console,
};

#[tauri::command]
pub(crate) fn get_mod_studio_sdk_schema(
    window: WebviewWindow,
) -> Result<ModStudioSdkSchemaSnapshot, CommandError> {
    console::validate_window(&window)?;
    Ok(ModStudioSdkSchemaSnapshot::current())
}

#[tauri::command]
pub(crate) async fn create_mod_studio_document(
    id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<ModStudioDocumentSnapshot, CommandError> {
    console::validate_window(&window)?;
    let service = state.mod_studio();
    tauri::async_runtime::spawn_blocking(move || service.create_document(&id))
        .await
        .map_err(|error| {
            log::error!("Mod document create task failed: {error}");
            CommandError::mod_workspace_task_failed()
        })?
        .map(Into::into)
        .map_err(|error| {
            log::error!("Mod document create failed: {}", error.detail);
            CommandError::from_mod_studio(error)
        })
}

#[tauri::command]
pub(crate) async fn open_mod_studio_folder(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<bool, CommandError> {
    console::validate_window(&window)?;
    let service = state.mod_studio();
    tauri::async_runtime::spawn_blocking(move || {
        service.load_workspace().map_err(|error| {
            log::error!("prepare Mod folder failed: {}", error.detail);
            CommandError::from_mod_studio(error)
        })?;
        #[cfg(windows)]
        {
            nte_dps_tool::platform::file_dialog::open_directory(
                &service.workspace_directory().join("nte-mods"),
            )
            .map_err(|error| {
                log::error!("open Mod folder failed: {error}");
                CommandError::mod_studio_folder_open_failed()
            })?;
            Ok(true)
        }
        #[cfg(not(windows))]
        {
            Err(CommandError::mod_studio_folder_open_failed())
        }
    })
    .await
    .map_err(|error| {
        log::error!("open Mod folder task failed: {error}");
        CommandError::mod_workspace_task_failed()
    })?
}

#[tauri::command]
pub(crate) async fn get_mod_studio_deployment(
    region: Option<String>,
    game_directory: Option<String>,
    window: WebviewWindow,
) -> Result<ModStudioDeploymentSnapshot, CommandError> {
    console::validate_window(&window)?;
    let manual = parse_manual_game_directory(region, game_directory)?;
    tauri::async_runtime::spawn_blocking(move || inspect_deployment(manual.as_ref()))
        .await
        .map_err(|error| {
            log::error!("Mod loader inspection task failed: {error}");
            CommandError::mod_workspace_task_failed()
        })?
        .map(Into::into)
        .map_err(deployment_error)
}

#[tauri::command]
pub(crate) async fn choose_mod_studio_game_directory(
    region: String,
    window: WebviewWindow,
) -> Result<ModStudioDirectorySelectionSnapshot, CommandError> {
    console::validate_window(&window)?;
    let region = parse_region(&region)?;
    #[cfg(windows)]
    {
        use nte_dps_tool::platform::file_dialog::{FolderDialogOutcome, choose_folder};

        let owner = window
            .hwnd()
            .map_err(|_| CommandError::mod_studio_file_dialog_failed())?
            .0 as isize;
        let title = i18n::t("Select game installation folder");
        tauri::async_runtime::spawn_blocking(move || match choose_folder(owner, &title) {
            Ok(FolderDialogOutcome::Selected(path)) => {
                let deployment =
                    inspect_deployment(Some(&(region, path.clone()))).map_err(deployment_error)?;
                Ok(ModStudioDirectorySelectionSnapshot {
                    selected: true,
                    path: Some(path.to_string_lossy().into_owned()),
                    deployment: deployment.into(),
                })
            }
            Ok(FolderDialogOutcome::Cancelled) => Ok(ModStudioDirectorySelectionSnapshot {
                selected: false,
                path: None,
                deployment: inspect_deployment(None).map_err(deployment_error)?.into(),
            }),
            Err(code) => {
                log::error!("native game directory dialog failed: {code:#010x}");
                Err(CommandError::mod_studio_file_dialog_failed())
            }
        })
        .await
        .map_err(|error| {
            log::error!("game directory selection task failed: {error}");
            CommandError::mod_workspace_task_failed()
        })?
    }
    #[cfg(not(windows))]
    {
        let _ = region;
        Err(CommandError::mod_studio_file_dialog_failed())
    }
}

#[tauri::command]
pub(crate) async fn set_mod_studio_loader_enabled(
    region: String,
    enabled: bool,
    game_directory: Option<String>,
    window: WebviewWindow,
) -> Result<ModStudioDeploymentSnapshot, CommandError> {
    console::validate_window(&window)?;
    let region = parse_region(&region)?;
    let directory = game_directory.map(PathBuf::from);
    tauri::async_runtime::spawn_blocking(move || {
        if enabled {
            let plugin =
                current_plugin()?.ok_or(ModsPluginDeploymentError::PluginSourceNotFound)?;
            install_mods_plugin_with_manual(region, &plugin, directory.as_deref())
        } else {
            remove_mods_plugin_with_manual(region, directory.as_deref())
        }
    })
    .await
    .map_err(|error| {
        log::error!("Mod loader update task failed: {error}");
        CommandError::mod_workspace_task_failed()
    })?
    .map(Into::into)
    .map_err(deployment_error)
}

fn inspect_deployment(
    manual: Option<&(ModsPluginGameRegion, PathBuf)>,
) -> Result<
    nte_dps_tool::platform::mods_plugin::ModsPluginDeploymentStatus,
    ModsPluginDeploymentError,
> {
    let plugin = current_plugin()?;
    inspect_plugin_deployment_with_manual(
        plugin.as_deref(),
        manual.map(|(region, path)| (*region, path.as_path())),
    )
}

fn current_plugin() -> Result<Option<Vec<u8>>, ModsPluginDeploymentError> {
    read_mods_plugin().map_err(|error| ModsPluginDeploymentError::FileSystem(error.to_string()))
}

fn parse_manual_game_directory(
    region: Option<String>,
    game_directory: Option<String>,
) -> Result<Option<(ModsPluginGameRegion, PathBuf)>, CommandError> {
    match (region, game_directory) {
        (None, None) => Ok(None),
        (Some(region), Some(directory)) if !directory.trim().is_empty() => Ok(Some((
            parse_region(&region)?,
            Path::new(&directory).to_path_buf(),
        ))),
        _ => Err(CommandError::from_mod_studio_deployment(
            ModsPluginDeploymentError::InvalidGameDirectory,
        )),
    }
}

fn parse_region(region: &str) -> Result<ModsPluginGameRegion, CommandError> {
    match region {
        "china" => Ok(ModsPluginGameRegion::China),
        "global" => Ok(ModsPluginGameRegion::Global),
        _ => Err(CommandError::from_mod_studio_deployment(
            ModsPluginDeploymentError::GameInstallationNotFound,
        )),
    }
}

fn deployment_error(error: ModsPluginDeploymentError) -> CommandError {
    let category = match &error {
        ModsPluginDeploymentError::GameRunning => "game_running",
        ModsPluginDeploymentError::GameProcessProbe(_) => "game_probe",
        ModsPluginDeploymentError::GameInstallationNotFound => "game_not_found",
        ModsPluginDeploymentError::InvalidGameDirectory => "invalid_game_directory",
        ModsPluginDeploymentError::Registry(_) => "registry",
        ModsPluginDeploymentError::PluginSourceNotFound => "source_not_found",
        ModsPluginDeploymentError::ConflictingDwmapi => "conflicting_dwmapi",
        ModsPluginDeploymentError::InstalledPluginChanged => "installed_plugin_changed",
        ModsPluginDeploymentError::FileSystem(_) => "file_system",
    };
    log::error!("Mod loader operation failed in category {category}");
    CommandError::from_mod_studio_deployment(error)
}

#[tauri::command]
pub(crate) async fn get_mod_studio_workspace(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<ModStudioWorkspaceSnapshot, CommandError> {
    console::validate_window(&window)?;
    let service = state.mod_studio();
    tauri::async_runtime::spawn_blocking(move || service.load_workspace())
        .await
        .map_err(|error| {
            log::error!("Mod workspace list task failed: {error}");
            CommandError::mod_workspace_task_failed()
        })?
        .map(Into::into)
        .map_err(|error| {
            log::error!("Mod workspace list failed: {}", error.detail);
            CommandError::from_mod_studio(error)
        })
}

#[tauri::command]
pub(crate) async fn get_mod_studio_document(
    id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<ModStudioDocumentSnapshot, CommandError> {
    console::validate_window(&window)?;
    let service = state.mod_studio();
    tauri::async_runtime::spawn_blocking(move || service.load_document(&id))
        .await
        .map_err(|error| {
            log::error!("Mod document task failed: {error}");
            CommandError::mod_workspace_task_failed()
        })?
        .map(Into::into)
        .map_err(|error| {
            log::error!("Mod document load failed: {}", error.detail);
            CommandError::from_mod_studio(error)
        })
}

#[tauri::command]
pub(crate) async fn save_mod_studio_document(
    id: String,
    source: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<ModStudioDocumentSnapshot, CommandError> {
    console::validate_window(&window)?;
    let service = state.mod_studio();
    tauri::async_runtime::spawn_blocking(move || service.save_document(&id, &source))
        .await
        .map_err(|error| {
            log::error!("Mod document save task failed: {error}");
            CommandError::mod_workspace_task_failed()
        })?
        .map(Into::into)
        .map_err(|error| {
            log::error!("Mod document save failed: {}", error.detail);
            CommandError::from_mod_studio(error)
        })
}

#[tauri::command]
pub(crate) async fn set_mod_studio_document_enabled(
    id: String,
    enabled: bool,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<ModStudioWorkspaceSnapshot, CommandError> {
    console::validate_window(&window)?;
    let service = state.mod_studio();
    tauri::async_runtime::spawn_blocking(move || service.set_document_enabled(&id, enabled))
        .await
        .map_err(|error| {
            log::error!("Mod enabled-set task failed: {error}");
            CommandError::mod_workspace_task_failed()
        })?
        .map(Into::into)
        .map_err(|error| {
            log::error!("Mod enabled-set update failed: {}", error.detail);
            CommandError::from_mod_studio(error)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_game_directory_requires_a_known_region_and_nonempty_path() {
        assert!(
            parse_manual_game_directory(Some("global".to_owned()), Some(r"D:\\Game".to_owned()))
                .is_ok()
        );
        assert!(
            parse_manual_game_directory(Some("future".to_owned()), Some("x".to_owned())).is_err()
        );
        assert!(parse_manual_game_directory(None, Some("x".to_owned())).is_err());
        assert!(
            parse_manual_game_directory(Some("china".to_owned()), Some("  ".to_owned())).is_err()
        );
    }
}
