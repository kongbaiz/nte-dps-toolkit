use tauri::WebviewWindow;

use crate::contract::CommandError;

pub(crate) const ABYSS_VALUES_WINDOW_LABEL: &str = "abyss-values";

pub(crate) fn validate_window(window: &WebviewWindow) -> Result<(), CommandError> {
    if window.label() == ABYSS_VALUES_WINDOW_LABEL {
        Ok(())
    } else {
        Err(CommandError::invalid_window())
    }
}

pub(crate) fn show(window: &WebviewWindow) -> Result<(), CommandError> {
    window
        .set_title(&nte_dps_tool::storage::i18n::t("Abyss monster values"))
        .map_err(window_error)?;
    window.show().map_err(window_error)?;
    window.unminimize().map_err(window_error)?;
    window.set_focus().map_err(window_error)
}

fn window_error(error: tauri::Error) -> CommandError {
    log::error!("open abyss values window failed: {error}");
    CommandError::window_operation_failed()
}
