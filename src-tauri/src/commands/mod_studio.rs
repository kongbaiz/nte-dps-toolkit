use crate::{
    contract::{
        CommandError,
        mod_studio::{ModMarketCatalogSnapshot, ModMarketLocalStateSnapshot},
    },
    state::AppState,
    windows::console,
};
use nte_dps_tool::{
    core::mod_market::{
        MAX_MOD_MARKET_CATALOG_BYTES, MOD_MARKET_CATALOG_URL, find_mod_market_item,
        mod_market_package_is_current, parse_mod_market_catalog, verify_mod_market_package,
    },
    platform::update_http,
};
use tauri::{State, WebviewWindow};

#[tauri::command]
pub(crate) async fn get_mod_market_catalog(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<ModMarketCatalogSnapshot, CommandError> {
    console::validate_window(&window)?;
    let directory = super::toolkit::plugin_directory(state.inner()).ok();
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = fetch_mod_market_catalog()?;
        Ok(ModMarketCatalogSnapshot::from_catalog(
            catalog,
            |item| match directory.as_ref().map_or(Ok(None), |dir| {
                nte_dps_tool::core::mod_market::read_installed_plugin(dir, &item.id)
            }) {
                Ok(Some(bytes)) => ModMarketLocalStateSnapshot::Installed {
                    enabled: None,
                    current: mod_market_package_is_current(item, &bytes),
                },
                Ok(None) => ModMarketLocalStateSnapshot::NotInstalled,
                Err(_) => ModMarketLocalStateSnapshot::Unreadable {
                    code: "mod_workspace_read_failed",
                    message_key: "Failed to read the Mod workspace.",
                },
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
) -> Result<bool, CommandError> {
    console::validate_window(&window)?;
    if state.data_mode() != nte_dps_tool::core::toolkit::DataMode::Plugin {
        return Err(CommandError::mod_studio_risk_acknowledgement_required());
    }
    let directory = super::toolkit::plugin_directory(state.inner())?;
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = fetch_mod_market_catalog()?;
        let item = find_mod_market_item(&catalog, &id).map_err(CommandError::from_mod_market)?;
        let bytes = update_http::get_bytes(&item.package_url, item.package_size as usize)
            .map_err(|_| CommandError::mod_market_download_failed())?;
        let verified =
            verify_mod_market_package(item, &bytes).map_err(CommandError::from_mod_market)?;
        nte_dps_tool::core::mod_market::install_plugin(&directory, &id, &verified)
            .map_err(CommandError::from_mod_market)?;
        Ok(true)
    })
    .await
    .map_err(|_| CommandError::mod_workspace_task_failed())?
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
