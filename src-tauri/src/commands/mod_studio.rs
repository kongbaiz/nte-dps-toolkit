use std::path::{Path, PathBuf};

use nte_dps_tool::{
    core::{
        mod_market::{
            MAX_MOD_MARKET_CATALOG_BYTES, MOD_MARKET_CATALOG_URL, find_mod_market_item,
            mod_market_package_is_current, parse_mod_market_catalog, verify_mod_market_package,
        },
        mod_studio::ModStudioErrorCode,
    },
    platform::mods_plugin::{
        ModsPluginDeploymentError, ModsPluginGameRegion, inspect_plugin_deployment_with_manual,
        install_mods_plugin_with_manual, remove_mods_plugin_with_manual,
    },
    platform::update_http,
    storage::{config::MOD_STUDIO_GAME_DIRECTORY_MAX_BYTES, i18n, resource::read_mods_plugin},
};
use tauri::{State, WebviewWindow};

use crate::{
    contract::{
        CommandError,
        mod_studio::{
            ModMarketCatalogSnapshot, ModStudioDeploymentSnapshot,
            ModStudioDirectorySelectionSnapshot, ModStudioDocumentSnapshot,
            ModStudioGameDirectorySnapshot, ModStudioSdkSchemaSnapshot, ModStudioWorkspaceSnapshot,
        },
    },
    state::AppState,
    windows::console,
};

#[tauri::command]
pub(crate) async fn get_mod_market_catalog(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<ModMarketCatalogSnapshot, CommandError> {
    console::validate_window(&window)?;
    let service = state.mod_studio();
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = fetch_mod_market_catalog()?;
        Ok(ModMarketCatalogSnapshot::from_catalog(
            catalog,
            |item| match service.load_document(&item.id) {
                Ok(document) => (
                    true,
                    document.enabled,
                    mod_market_package_is_current(item, &document.source),
                ),
                Err(error) if error.code == ModStudioErrorCode::DocumentNotFound => {
                    (false, false, false)
                }
                Err(_) => (true, false, false),
            },
        ))
    })
    .await
    .map_err(|error| {
        log::error!("Mod Market catalog task failed: {error}");
        CommandError::mod_workspace_task_failed()
    })?
}

#[tauri::command]
pub(crate) async fn install_mod_market_item(
    id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<ModStudioDocumentSnapshot, CommandError> {
    console::validate_window(&window)?;
    let service = state.mod_studio();
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = fetch_mod_market_catalog()?;
        let item = find_mod_market_item(&catalog, &id).map_err(|error| {
            log::error!("Mod Market lookup failed in category {:?}", error.code);
            CommandError::from_mod_market(error)
        })?;
        let bytes = update_http::get_bytes(&item.package_url, item.package_size as usize).map_err(
            |error| {
                log::error!("Mod Market package download failed: {error}");
                CommandError::mod_market_download_failed()
            },
        )?;
        let source = verify_mod_market_package(item, &bytes).map_err(|error| {
            log::error!(
                "Mod Market package verification failed in category {:?}",
                error.code
            );
            CommandError::from_mod_market(error)
        })?;
        service
            .install_market_document(&item.id, &source)
            .map(Into::into)
            .map_err(|error| {
                log::error!(
                    "Mod Market workspace install failed in category {:?}",
                    error.code
                );
                CommandError::from_mod_studio(error)
            })
    })
    .await
    .map_err(|error| {
        log::error!("Mod Market install task failed: {error}");
        CommandError::mod_workspace_task_failed()
    })?
}

fn fetch_mod_market_catalog()
-> Result<nte_dps_tool::core::mod_market::ModMarketCatalog, CommandError> {
    let bytes = update_http::get_bytes(MOD_MARKET_CATALOG_URL, MAX_MOD_MARKET_CATALOG_BYTES)
        .map_err(|error| {
            log::error!("Mod Market catalog download failed: {error}");
            CommandError::mod_market_download_failed()
        })?;
    parse_mod_market_catalog(&bytes).map_err(|error| {
        log::error!(
            "Mod Market catalog validation failed in category {:?}",
            error.code
        );
        CommandError::from_mod_market(error)
    })
}

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
pub(crate) async fn delete_mod_studio_document(
    id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<ModStudioWorkspaceSnapshot, CommandError> {
    console::validate_window(&window)?;
    let service = state.mod_studio();
    tauri::async_runtime::spawn_blocking(move || service.delete_document(&id))
        .await
        .map_err(|error| {
            log::error!("Mod document delete task failed: {error}");
            CommandError::mod_workspace_task_failed()
        })?
        .map(Into::into)
        .map_err(|error| {
            log::error!("Mod document delete failed in category {:?}", error.code);
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
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<ModStudioDeploymentSnapshot, CommandError> {
    console::validate_window(&window)?;
    let manual = resolve_manual_game_directory(state.inner(), region, game_directory)?;
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
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<ModStudioDirectorySelectionSnapshot, CommandError> {
    console::validate_window(&window)?;
    let region = parse_region(&region)?;
    let state = state.inner().clone();
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
                let path = path.to_string_lossy().into_owned();
                state
                    .set_mod_studio_game_directory(region, Some(path.clone()))
                    .map_err(|error| {
                        log::error!("save Mod Studio game directory preference failed: {error}");
                        CommandError::settings_config_save_failed()
                    })?;
                Ok(ModStudioDirectorySelectionSnapshot {
                    selected: true,
                    path: Some(path),
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
        let _ = (region, state);
        Err(CommandError::mod_studio_file_dialog_failed())
    }
}

#[tauri::command]
pub(crate) fn get_mod_studio_game_directory(
    region: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<ModStudioGameDirectorySnapshot, CommandError> {
    console::validate_window(&window)?;
    let region = parse_region(&region)?;
    Ok(ModStudioGameDirectorySnapshot {
        contract_version: crate::contract::mod_studio::MOD_STUDIO_DIRECTORY_CONTRACT_VERSION,
        region: region.into(),
        path: state.mod_studio_game_directory(region),
    })
}

#[tauri::command]
pub(crate) fn set_mod_studio_game_directory(
    region: String,
    game_directory: Option<String>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<ModStudioGameDirectorySnapshot, CommandError> {
    console::validate_window(&window)?;
    let region = parse_region(&region)?;
    let directory = match game_directory {
        Some(directory) => Some(
            parse_manual_game_directory(Some(region_name(region).to_owned()), Some(directory))?
                .ok_or_else(|| {
                    CommandError::from_mod_studio_deployment(
                        ModsPluginDeploymentError::InvalidGameDirectory,
                    )
                })?
                .1
                .to_string_lossy()
                .into_owned(),
        ),
        None => None,
    };
    state
        .set_mod_studio_game_directory(region, directory.clone())
        .map_err(|error| {
            log::error!("save Mod Studio game directory preference failed: {error}");
            CommandError::settings_config_save_failed()
        })?;
    Ok(ModStudioGameDirectorySnapshot {
        contract_version: crate::contract::mod_studio::MOD_STUDIO_DIRECTORY_CONTRACT_VERSION,
        region: region.into(),
        path: directory,
    })
}

#[tauri::command]
pub(crate) async fn set_mod_studio_loader_enabled(
    region: String,
    enabled: bool,
    game_directory: Option<String>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<ModStudioDeploymentSnapshot, CommandError> {
    console::validate_window(&window)?;
    let region = parse_region(&region)?;
    let directory = resolve_manual_game_directory(
        state.inner(),
        Some(region_name(region).to_owned()),
        game_directory,
    )?
    .map(|(_, directory)| directory);
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
        (Some(region), Some(directory))
            if !directory.trim().is_empty()
                && directory.len() <= MOD_STUDIO_GAME_DIRECTORY_MAX_BYTES
                && !directory
                    .chars()
                    .any(|character| matches!(character, '\0' | '\r' | '\n')) =>
        {
            Ok(Some((
                parse_region(&region)?,
                Path::new(directory.trim()).to_path_buf(),
            )))
        }
        _ => Err(CommandError::from_mod_studio_deployment(
            ModsPluginDeploymentError::InvalidGameDirectory,
        )),
    }
}

fn resolve_manual_game_directory(
    state: &AppState,
    region: Option<String>,
    game_directory: Option<String>,
) -> Result<Option<(ModsPluginGameRegion, PathBuf)>, CommandError> {
    match (region, game_directory) {
        (Some(region), None) => {
            let region = parse_region(&region)?;
            Ok(state
                .mod_studio_game_directory(region)
                .map(|directory| (region, PathBuf::from(directory))))
        }
        (region, game_directory) => parse_manual_game_directory(region, game_directory),
    }
}

fn region_name(region: ModsPluginGameRegion) -> &'static str {
    match region {
        ModsPluginGameRegion::China => "china",
        ModsPluginGameRegion::Global => "global",
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
        assert!(
            parse_manual_game_directory(
                Some("china".to_owned()),
                Some("x".repeat(MOD_STUDIO_GAME_DIRECTORY_MAX_BYTES + 1)),
            )
            .is_err()
        );
    }
}
