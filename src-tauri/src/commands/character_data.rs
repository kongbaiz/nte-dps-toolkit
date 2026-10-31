use nte_dps_tool::core::character_data::CharacterDataError;
use tauri::{State, WebviewWindow};

use crate::{
    character_data_service::CharacterDataServiceError,
    contract::{CommandError, character_data::CharacterDataSnapshot},
    state::AppState,
    windows,
};

#[tauri::command]
pub(crate) fn get_character_data_snapshot(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<CharacterDataSnapshot, CommandError> {
    windows::validate_character_avatar_window(&window)?;
    let projection = state
        .character_data_snapshot()
        .map_err(character_data_error)?;
    Ok(CharacterDataSnapshot::from_projection(projection))
}

fn character_data_error(error: CharacterDataServiceError) -> CommandError {
    match error {
        CharacterDataServiceError::Busy => CommandError::character_data(
            "character_data_busy",
            "Another character data operation is in progress.",
            Vec::new(),
        ),
        CharacterDataServiceError::Domain(error) => character_data_domain_error(error),
    }
}

fn character_data_domain_error(error: CharacterDataError) -> CommandError {
    match error {
        CharacterDataError::InvalidId => CommandError::character_data(
            "character_id_invalid",
            "Character ID must be a positive integer.",
            Vec::new(),
        ),
        CharacterDataError::TooManyRecords => CommandError::character_data(
            "character_table_too_large",
            "Character table has too many entries.",
            Vec::new(),
        ),
        CharacterDataError::Read(_)
        | CharacterDataError::Json(_)
        | CharacterDataError::MissingCharacters
        | CharacterDataError::InvalidRecord(_) => {
            log::error!("load Console character data failed");
            CommandError::character_data(
                "character_data_load_failed",
                "Character data could not be loaded.",
                Vec::new(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_errors_map_to_stable_contract_codes() {
        let invalid_id = character_data_domain_error(CharacterDataError::InvalidId);
        assert_eq!(invalid_id.code, "character_id_invalid");
        assert_eq!(
            invalid_id.message_key,
            "Character ID must be a positive integer."
        );
    }

    #[test]
    fn service_state_errors_are_stable_and_redacted() {
        let busy = character_data_error(CharacterDataServiceError::Busy);
        assert_eq!(busy.code, "character_data_busy");
        assert!(busy.diagnostic_line.is_none());
    }
}
