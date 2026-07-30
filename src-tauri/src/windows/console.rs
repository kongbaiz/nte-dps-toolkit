use tauri::WebviewWindow;

use crate::contract::CommandError;

pub(crate) const CONSOLE_WINDOW_LABEL: &str = "console";

pub(crate) fn validate_window(window: &WebviewWindow) -> Result<(), CommandError> {
    if window.label() == CONSOLE_WINDOW_LABEL {
        Ok(())
    } else {
        Err(CommandError::invalid_window())
    }
}
