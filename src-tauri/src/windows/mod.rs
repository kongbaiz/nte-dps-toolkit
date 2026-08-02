pub(crate) mod abyss_values;
pub(crate) mod combat_details;
pub(crate) mod console;
pub(crate) mod hud;
pub(crate) mod island;
pub(crate) mod main_dps;
#[cfg(windows)]
pub(crate) mod passthrough_hotkey;

use tauri::WebviewWindow;

use crate::contract::CommandError;

pub(crate) const WINDOW_MOTION_ENTER_EVENT: &str = "desktop-window-motion-enter";

pub(crate) fn validate_character_avatar_window(window: &WebviewWindow) -> Result<(), CommandError> {
    if is_character_avatar_window_label(window.label()) {
        Ok(())
    } else {
        Err(CommandError::invalid_window())
    }
}

fn is_character_avatar_window_label(label: &str) -> bool {
    matches!(
        label,
        main_dps::MAIN_DPS_WINDOW_LABEL
            | hud::HUD_WINDOW_LABEL
            | console::CONSOLE_WINDOW_LABEL
            | abyss_values::ABYSS_VALUES_WINDOW_LABEL
            | combat_details::CHARACTER_DETAILS_WINDOW_LABEL
            | combat_details::TEAM_DETAILS_WINDOW_LABEL
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatar_catalog_is_available_only_to_avatar_consuming_windows() {
        for label in [
            main_dps::MAIN_DPS_WINDOW_LABEL,
            hud::HUD_WINDOW_LABEL,
            console::CONSOLE_WINDOW_LABEL,
            abyss_values::ABYSS_VALUES_WINDOW_LABEL,
            combat_details::CHARACTER_DETAILS_WINDOW_LABEL,
            combat_details::TEAM_DETAILS_WINDOW_LABEL,
        ] {
            assert!(is_character_avatar_window_label(label), "{label}");
        }
        assert!(!is_character_avatar_window_label(
            island::ISLAND_WINDOW_LABEL
        ));
    }
}
