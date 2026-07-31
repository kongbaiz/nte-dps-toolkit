use nte_dps_tool::storage::config::{HUD_WIDTH_MAX, HUD_WIDTH_MIN, HudModule};

use crate::contract::CommandError;

pub(crate) mod abyss_values;
pub(crate) mod mod_studio;
pub(crate) mod settings;
pub(crate) mod technical;

pub(crate) fn parse_hud_module(module: &str) -> Result<HudModule, CommandError> {
    match module {
        "title" => Ok(HudModule::Title),
        "summary" => Ok(HudModule::Summary),
        "status" => Ok(HudModule::Status),
        "characters" => Ok(HudModule::Characters),
        "timeline" => Ok(HudModule::Timeline),
        _ => Err(CommandError::invalid_hud_module()),
    }
}

pub(crate) fn sanitize_hud_width(width: i32) -> u16 {
    width.clamp(i32::from(HUD_WIDTH_MIN), i32::from(HUD_WIDTH_MAX)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hud_module_input_accepts_only_stable_contract_values() {
        assert_eq!(parse_hud_module("title").expect("title"), HudModule::Title);
        assert_eq!(
            parse_hud_module("characters").expect("characters"),
            HudModule::Characters
        );
        assert_eq!(
            parse_hud_module("timeline").expect("timeline"),
            HudModule::Timeline
        );
        assert_eq!(
            parse_hud_module("status").expect("status"),
            HudModule::Status
        );
        assert!(parse_hud_module("future_module").is_err());
        assert!(parse_hud_module("../summary").is_err());
    }

    #[test]
    fn hud_width_input_uses_existing_config_bounds() {
        assert_eq!(sanitize_hud_width(i32::MIN), HUD_WIDTH_MIN);
        assert_eq!(sanitize_hud_width(512), 512);
        assert_eq!(sanitize_hud_width(i32::MAX), HUD_WIDTH_MAX);
    }
}
