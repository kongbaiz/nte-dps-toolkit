use nte_dps_tool::storage::config::{HUD_WIDTH_MAX, HUD_WIDTH_MIN};

pub(crate) mod abyss_values;
pub(crate) mod character_data;
pub(crate) mod console_control;
pub(crate) mod desktop_window;
pub(crate) mod diagnostics;
pub(crate) mod empty_curtain;
pub(crate) mod encrypted_ini;
pub(crate) mod history;
pub(crate) mod island;
pub(crate) mod main_dps;
pub(crate) mod mod_studio;
pub(crate) mod packets;
pub(crate) mod settings;
pub(crate) mod skills;
pub(crate) mod technical;
pub(crate) mod timeline;
pub(crate) mod toolkit;
pub(crate) mod user_characters;

pub(crate) fn sanitize_hud_width(width: i32) -> u16 {
    width.clamp(i32::from(HUD_WIDTH_MIN), i32::from(HUD_WIDTH_MAX)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hud_width_input_uses_existing_config_bounds() {
        assert_eq!(sanitize_hud_width(i32::MIN), HUD_WIDTH_MIN);
        assert_eq!(sanitize_hud_width(512), 512);
        assert_eq!(sanitize_hud_width(i32::MAX), HUD_WIDTH_MAX);
    }
}
