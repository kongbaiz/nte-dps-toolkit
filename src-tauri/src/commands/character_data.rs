use nte_dps_tool::core::character_data::CharacterDataError;
use tauri::{State, WebviewWindow};

use crate::{
    character_data_service::CharacterDataServiceError,
    contract::{
        CommandError,
        character_data::{CharacterDataSnapshot, SaveCharacterDataRecordRequest},
    },
    state::AppState,
    windows::{self, console},
};

#[tauri::command]
pub(crate) fn get_character_data_snapshot(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<CharacterDataSnapshot, CommandError> {
    windows::validate_character_avatar_window(&window)?;
    let (projection, generation) = state
        .character_data_snapshot()
        .map_err(character_data_error)?;
    Ok(CharacterDataSnapshot::from_projection(
        projection, generation,
    ))
}

#[tauri::command]
pub(crate) fn save_character_data_record(
    input: SaveCharacterDataRecordRequest,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<CharacterDataSnapshot, CommandError> {
    console::validate_window(&window)?;
    let (projection, generation) = state
        .save_character_data_record(input.into())
        .map_err(character_data_error)?;
    Ok(CharacterDataSnapshot::from_projection(
        projection, generation,
    ))
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
        CharacterDataError::IdAlreadyExists(id) => CommandError::character_data(
            "character_id_exists",
            "Character ID {} already exists.",
            vec![id.to_string()],
        ),
        CharacterDataError::RecordMissing(id) => CommandError::character_data(
            "character_record_missing",
            "Character ID {} no longer exists.",
            vec![id.to_string()],
        ),
        CharacterDataError::IdImmutable => CommandError::character_data(
            "character_id_immutable",
            "Existing character IDs cannot be changed.",
            Vec::new(),
        ),
        CharacterDataError::NameRequired => CommandError::character_data(
            "character_name_required",
            "Chinese or English name is required.",
            Vec::new(),
        ),
        CharacterDataError::InvalidColor => CommandError::character_data(
            "character_color_invalid",
            "Color must use #RRGGBB format.",
            Vec::new(),
        ),
        CharacterDataError::InvalidAttribute => CommandError::character_data(
            "character_attribute_invalid",
            "Character attribute is invalid.",
            Vec::new(),
        ),
        CharacterDataError::FieldTooLong(field) => CommandError::character_data(
            "character_field_too_long",
            "Character field {} is too long.",
            vec![field.to_owned()],
        ),
        CharacterDataError::TooManyRecords => CommandError::character_data(
            "character_table_too_large",
            "Character table has too many entries.",
            Vec::new(),
        ),
        CharacterDataError::Serialize(_) | CharacterDataError::Write(_) => {
            log::error!("save Console character data failed");
            CommandError::character_data(
                "character_data_save_failed",
                "Failed to save character data.",
                Vec::new(),
            )
        }
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

        let duplicate = character_data_domain_error(CharacterDataError::IdAlreadyExists(1080));
        assert_eq!(duplicate.code, "character_id_exists");
        assert_eq!(duplicate.message_arguments, ["1080"]);
    }

    #[test]
    fn service_state_errors_are_stable_and_redacted() {
        let busy = character_data_error(CharacterDataServiceError::Busy);
        assert_eq!(busy.code, "character_data_busy");
        assert!(busy.diagnostic_line.is_none());
    }
}
