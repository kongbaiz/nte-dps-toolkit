use std::fs;

use nte_dps_tool::{
    core::{
        empty_curtain::{
            RecommendedLoadoutError, build_drive_calculator_inventory, recommended_loadout,
        },
        snapshot::{
            CHARACTER_LOADOUT_MAX_JSON_BYTES, CharacterLoadoutError, export_character_loadout_json,
            parse_character_loadout_json, validate_character_loadout,
        },
    },
    engine::model::HtItemNetId,
    platform::mods_plugin::{ModsPluginOperation, ModsPluginPlacement, ModsPluginSubmitError},
    storage::{i18n, io_util::atomic_write_text},
};
use serde::Deserialize;
use tauri::{State, WebviewWindow};

use crate::{
    contract::{
        CommandError,
        empty_curtain::{
            EmptyCurtainFileResult, EmptyCurtainPositionSnapshot, EmptyCurtainSnapshot,
        },
    },
    state::AppState,
    windows::console,
};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ItemUidInput {
    slot: u32,
    serial: u32,
}

impl From<ItemUidInput> for HtItemNetId {
    fn from(value: ItemUidInput) -> Self {
        HtItemNetId {
            solt: value.slot,
            serial: value.serial,
        }
    }
}

#[tauri::command]
pub(crate) fn get_empty_curtain_snapshot(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<EmptyCurtainSnapshot, CommandError> {
    console::validate_window(&window)?;
    Ok(snapshot(state.inner()))
}

#[tauri::command]
pub(crate) fn get_empty_curtain_positions(
    item: ItemUidInput,
    character: ItemUidInput,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<Vec<EmptyCurtainPositionSnapshot>, CommandError> {
    console::validate_window(&window)?;
    let item_id = HtItemNetId::from(item);
    let character_id = HtItemNetId::from(character);
    state.with_empty_curtain(|items, characters, catalog| {
        let item = items
            .iter()
            .find(|item| item.id == item_id)
            .ok_or_else(item_unavailable)?;
        let character = characters
            .iter()
            .find(|character| character.net_id == character_id)
            .ok_or_else(character_unavailable)?;
        let positions = catalog
            .valid_module_positions(character.character_id, &item.item_id)
            .ok_or_else(|| {
                CommandError::empty_curtain(
                    "empty_curtain_position_unavailable",
                    "No compatible position is available for this drive module",
                    Vec::new(),
                )
            })?;
        Ok(positions
            .iter()
            .map(|position| EmptyCurtainPositionSnapshot {
                row: position.row,
                column: position.column,
            })
            .collect())
    })
}

#[tauri::command]
pub(crate) fn manage_empty_curtain_item(
    item: ItemUidInput,
    action: String,
    character: Option<ItemUidInput>,
    row: Option<i32>,
    column: Option<i32>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<EmptyCurtainSnapshot, CommandError> {
    console::validate_window(&window)?;
    let item_id = HtItemNetId::from(item);
    let operation = state.with_empty_curtain(|items, characters, catalog| {
        let item = items
            .iter()
            .find(|item| item.id == item_id)
            .ok_or_else(item_unavailable)?;
        let definition = catalog
            .items
            .get(&item.item_id)
            .ok_or_else(metadata_unavailable)?;
        match action.as_str() {
            "lock" => Ok((
                HtItemNetId::ZERO,
                ModsPluginOperation::SetItemLocked {
                    equipment: item.id,
                    locked: true,
                },
            )),
            "unlock" => Ok((
                HtItemNetId::ZERO,
                ModsPluginOperation::SetItemLocked {
                    equipment: item.id,
                    locked: false,
                },
            )),
            "discard" => Ok((
                HtItemNetId::ZERO,
                ModsPluginOperation::SetItemDiscarded {
                    equipment: item.id,
                    discarded: true,
                },
            )),
            "restore" => Ok((
                HtItemNetId::ZERO,
                ModsPluginOperation::SetItemDiscarded {
                    equipment: item.id,
                    discarded: false,
                },
            )),
            "unequip" => {
                let equipped = item.character_net_id.ok_or_else(item_not_equipped)?;
                let operation = match definition.kind {
                    nte_dps_tool::engine::parser::EquipmentKind::Module => {
                        ModsPluginOperation::UnequipModule { equipment: item.id }
                    }
                    nte_dps_tool::engine::parser::EquipmentKind::Core => {
                        ModsPluginOperation::UnequipCore { equipment: item.id }
                    }
                };
                Ok((equipped, operation))
            }
            "equip" => {
                let target_uid = character.ok_or_else(character_unavailable)?.into();
                let target = characters
                    .iter()
                    .find(|candidate| candidate.net_id == target_uid)
                    .ok_or_else(character_unavailable)?;
                let moving = item
                    .character_net_id
                    .is_some_and(|equipped| equipped != target.net_id);
                let operation = match definition.kind {
                    nte_dps_tool::engine::parser::EquipmentKind::Core if moving => {
                        ModsPluginOperation::MoveCoreToCharacter { equipment: item.id }
                    }
                    nte_dps_tool::engine::parser::EquipmentKind::Core => {
                        ModsPluginOperation::EquipCore { equipment: item.id }
                    }
                    nte_dps_tool::engine::parser::EquipmentKind::Module => {
                        let row = row.ok_or_else(position_required)?;
                        let column = column.ok_or_else(position_required)?;
                        let positions = catalog
                            .valid_module_positions(target.character_id, &item.item_id)
                            .ok_or_else(position_required)?;
                        if !positions
                            .iter()
                            .any(|position| position.row == row && position.column == column)
                        {
                            return Err(position_required());
                        }
                        if moving {
                            ModsPluginOperation::MoveModuleToCharacter {
                                equipment: item.id,
                                row,
                                column,
                            }
                        } else {
                            ModsPluginOperation::EquipModule {
                                equipment: item.id,
                                row,
                                column,
                            }
                        }
                    }
                };
                Ok((target.net_id, operation))
            }
            _ => Err(CommandError::empty_curtain(
                "empty_curtain_action_invalid",
                "Equipment action is invalid",
                Vec::new(),
            )),
        }
    })?;
    submit(state.inner(), operation.0, operation.1)?;
    Ok(snapshot(state.inner()))
}

#[tauri::command]
pub(crate) fn apply_empty_curtain_character_action(
    character: ItemUidInput,
    action: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<EmptyCurtainSnapshot, CommandError> {
    console::validate_window(&window)?;
    let character_uid = HtItemNetId::from(character);
    let operation = state.with_empty_curtain(|items, characters, catalog| {
        let character = characters
            .iter()
            .copied()
            .find(|candidate| candidate.net_id == character_uid)
            .ok_or_else(character_unavailable)?;
        match action.as_str() {
            "unequip-all" => Ok(ModsPluginOperation::UnequipAll),
            "one-click" => recommended_loadout(character, items, catalog)
                .map(|loadout| ModsPluginOperation::EquipOneKey {
                    placements: loadout
                        .placements
                        .into_iter()
                        .map(|placement| ModsPluginPlacement {
                            equipment: placement.equipment,
                            row: placement.row,
                            column: placement.column,
                        })
                        .collect(),
                    core: loadout.core,
                })
                .map_err(recommended_loadout_error),
            _ => Err(CommandError::empty_curtain(
                "empty_curtain_character_action_invalid",
                "Character equipment action is invalid",
                Vec::new(),
            )),
        }
    })?;
    submit(state.inner(), character_uid, operation)?;
    Ok(snapshot(state.inner()))
}

#[tauri::command]
pub(crate) async fn export_empty_curtain_inventory(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<EmptyCurtainFileResult, CommandError> {
    console::validate_window(&window)?;
    let json = state.with_empty_curtain(|items, _, catalog| {
        if items.is_empty() {
            return Err(CommandError::empty_curtain(
                "empty_curtain_inventory_empty",
                "No Console equipment to export",
                Vec::new(),
            ));
        }
        serde_json::to_string_pretty(&build_drive_calculator_inventory(items, catalog)).map_err(
            |error| {
                log::error!("serialize Drive Calculator inventory failed: {error}");
                CommandError::empty_curtain(
                    "empty_curtain_export_serialize_failed",
                    "Failed to serialize Console equipment",
                    Vec::new(),
                )
            },
        )
    })?;
    let completed = save_json(
        &window,
        i18n::t("Drive Calculator inventory"),
        "real_inventory.json",
        json,
    )
    .await?;
    Ok(EmptyCurtainFileResult {
        completed,
        snapshot: snapshot(state.inner()),
    })
}

#[tauri::command]
pub(crate) async fn export_empty_curtain_loadout(
    character: ItemUidInput,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<EmptyCurtainFileResult, CommandError> {
    console::validate_window(&window)?;
    let character_uid = HtItemNetId::from(character);
    let (json, character_id) = state.with_empty_curtain(|items, characters, catalog| {
        let character = characters
            .iter()
            .copied()
            .find(|candidate| candidate.net_id == character_uid)
            .ok_or_else(character_unavailable)?;
        export_character_loadout_json(character, items, catalog)
            .map(|json| (json, character.character_id))
            .map_err(character_loadout_error)
    })?;
    let completed = save_json(
        &window,
        i18n::t("Console character loadout"),
        &format!("nte_loadout_{character_id}.json"),
        json,
    )
    .await?;
    Ok(EmptyCurtainFileResult {
        completed,
        snapshot: snapshot(state.inner()),
    })
}

#[tauri::command]
pub(crate) async fn import_empty_curtain_loadout(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<EmptyCurtainFileResult, CommandError> {
    console::validate_window(&window)?;
    let Some(json) = open_json(&window, i18n::t("Console character loadout")).await? else {
        return Ok(EmptyCurtainFileResult {
            completed: false,
            snapshot: snapshot(state.inner()),
        });
    };
    let operation = state.with_empty_curtain(|items, characters, catalog| {
        let file = parse_character_loadout_json(&json).map_err(character_loadout_error)?;
        let loadout = validate_character_loadout(&file, characters, items, catalog)
            .map_err(character_loadout_error)?;
        Ok((
            loadout.character.net_id,
            ModsPluginOperation::EquipOneKey {
                placements: loadout
                    .placements
                    .into_iter()
                    .map(|placement| ModsPluginPlacement {
                        equipment: placement.equipment,
                        row: placement.row,
                        column: placement.column,
                    })
                    .collect(),
                core: loadout.core,
            },
        ))
    })?;
    submit(state.inner(), operation.0, operation.1)?;
    Ok(EmptyCurtainFileResult {
        completed: true,
        snapshot: snapshot(state.inner()),
    })
}

pub(crate) fn snapshot(state: &AppState) -> EmptyCurtainSnapshot {
    let inventory = state.empty_curtain_snapshot();
    let catalog = state.equipment_catalog();
    let resources = state.live_capture_resources();
    EmptyCurtainSnapshot::from_inventory(
        inventory,
        &catalog,
        &resources.characters,
        state.empty_curtain_operation(),
    )
}

fn submit(
    state: &AppState,
    character: HtItemNetId,
    operation: ModsPluginOperation,
) -> Result<(), CommandError> {
    state
        .submit_empty_curtain_operation(character, operation)
        .map(|_| ())
        .map_err(|ModsPluginSubmitError::Busy| {
            CommandError::empty_curtain(
                "empty_curtain_operation_busy",
                "Mod loader is busy; try again shortly",
                Vec::new(),
            )
        })
}

async fn save_json(
    window: &WebviewWindow,
    title: String,
    default_file_name: &str,
    json: String,
) -> Result<bool, CommandError> {
    #[cfg(windows)]
    {
        use nte_dps_tool::platform::file_dialog::{SaveFileDialogOutcome, choose_json_save_path};
        let owner = window.hwnd().map_err(|_| file_dialog_error())?.0 as isize;
        let default_file_name = default_file_name.to_owned();
        return tauri::async_runtime::spawn_blocking(move || {
            match choose_json_save_path(owner, &title, &default_file_name) {
                Ok(SaveFileDialogOutcome::Selected(path)) => atomic_write_text(&path, &json)
                    .map(|_| true)
                    .map_err(|_| file_write_error()),
                Ok(SaveFileDialogOutcome::Cancelled) => Ok(false),
                Err(code) => {
                    log::error!("native Console equipment save dialog failed: {code:#010x}");
                    Err(file_dialog_error())
                }
            }
        })
        .await
        .map_err(|_| file_dialog_error())?;
    }
    #[cfg(not(windows))]
    {
        let _ = (window, title, default_file_name, json);
        Err(file_dialog_error())
    }
}

async fn open_json(window: &WebviewWindow, title: String) -> Result<Option<String>, CommandError> {
    #[cfg(windows)]
    {
        use nte_dps_tool::platform::file_dialog::{OpenFileDialogOutcome, choose_json_open_path};
        let owner = window.hwnd().map_err(|_| file_dialog_error())?.0 as isize;
        return tauri::async_runtime::spawn_blocking(move || {
            match choose_json_open_path(owner, &title) {
                Ok(OpenFileDialogOutcome::Selected(path)) => {
                    let metadata = fs::metadata(&path).map_err(|_| file_read_error())?;
                    if metadata.len() > CHARACTER_LOADOUT_MAX_JSON_BYTES as u64 {
                        return Err(character_loadout_error(CharacterLoadoutError::JsonTooLarge));
                    }
                    fs::read_to_string(path)
                        .map(Some)
                        .map_err(|_| file_read_error())
                }
                Ok(OpenFileDialogOutcome::Cancelled) => Ok(None),
                Err(code) => {
                    log::error!("native Console equipment open dialog failed: {code:#010x}");
                    Err(file_dialog_error())
                }
            }
        })
        .await
        .map_err(|_| file_dialog_error())?;
    }
    #[cfg(not(windows))]
    {
        let _ = (window, title);
        Err(file_dialog_error())
    }
}

fn character_loadout_error(error: CharacterLoadoutError) -> CommandError {
    let (code, key, arguments) = match error {
        CharacterLoadoutError::JsonTooLarge => (
            "empty_curtain_loadout_too_large",
            "Character loadout file is too large",
            Vec::new(),
        ),
        CharacterLoadoutError::InvalidJson => (
            "empty_curtain_loadout_invalid",
            "Character loadout JSON is invalid",
            Vec::new(),
        ),
        CharacterLoadoutError::UnsupportedVersion(version) => (
            "empty_curtain_loadout_version",
            "Unsupported character loadout version: {}",
            vec![version.to_string()],
        ),
        CharacterLoadoutError::CharacterUnavailable(character) => (
            "empty_curtain_character_unavailable",
            "Character {} is not available in the current session",
            vec![character.to_string()],
        ),
        CharacterLoadoutError::MissingCharacterPlan(_) => (
            "empty_curtain_plan_unavailable",
            "No character equipment template is available",
            Vec::new(),
        ),
        CharacterLoadoutError::MissingCore => (
            "empty_curtain_loadout_missing_core",
            "Character loadout has no cassette",
            Vec::new(),
        ),
        CharacterLoadoutError::MultipleCores => (
            "empty_curtain_loadout_multiple_cores",
            "Character loadout has multiple cassettes",
            Vec::new(),
        ),
        CharacterLoadoutError::MissingModules => (
            "empty_curtain_loadout_missing_modules",
            "Character loadout has no drive modules",
            Vec::new(),
        ),
        CharacterLoadoutError::TooManyModules => (
            "empty_curtain_loadout_too_many_modules",
            "Character loadout has too many drive modules",
            Vec::new(),
        ),
        CharacterLoadoutError::OverlappingModules => (
            "empty_curtain_loadout_overlap",
            "Character loadout contains overlapping drive modules",
            Vec::new(),
        ),
        other => (
            "empty_curtain_loadout_item_invalid",
            "Character loadout contains invalid equipment data: {}",
            vec![format!("{other:?}")],
        ),
    };
    CommandError::empty_curtain(code, key, arguments)
}

fn recommended_loadout_error(error: RecommendedLoadoutError) -> CommandError {
    match error {
        RecommendedLoadoutError::MissingTemplate => CommandError::empty_curtain(
            "empty_curtain_plan_unavailable",
            "No character equipment template is available",
            Vec::new(),
        ),
        RecommendedLoadoutError::MissingEquipment => CommandError::empty_curtain(
            "empty_curtain_plan_equipment_missing",
            "Required equipment for the character template is unavailable",
            Vec::new(),
        ),
    }
}

fn item_unavailable() -> CommandError {
    CommandError::empty_curtain(
        "empty_curtain_item_unavailable",
        "Equipment is no longer present in the current inventory",
        Vec::new(),
    )
}

fn character_unavailable() -> CommandError {
    CommandError::empty_curtain(
        "empty_curtain_character_unavailable",
        "Character is no longer available in the current session",
        Vec::new(),
    )
}

fn metadata_unavailable() -> CommandError {
    CommandError::empty_curtain(
        "empty_curtain_metadata_unavailable",
        "Equipment metadata is unavailable",
        Vec::new(),
    )
}

fn item_not_equipped() -> CommandError {
    CommandError::empty_curtain(
        "empty_curtain_item_not_equipped",
        "Equipment is not currently equipped",
        Vec::new(),
    )
}

fn position_required() -> CommandError {
    CommandError::empty_curtain(
        "empty_curtain_position_required",
        "Choose a compatible drive module position",
        Vec::new(),
    )
}

fn file_dialog_error() -> CommandError {
    CommandError::empty_curtain(
        "empty_curtain_file_dialog_failed",
        "Console equipment file dialog failed",
        Vec::new(),
    )
}

fn file_write_error() -> CommandError {
    CommandError::empty_curtain(
        "empty_curtain_file_write_failed",
        "Failed to write Console equipment file",
        Vec::new(),
    )
}

fn file_read_error() -> CommandError {
    CommandError::empty_curtain(
        "empty_curtain_file_read_failed",
        "Failed to read character loadout",
        Vec::new(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uid_input_preserves_both_network_fields() {
        assert_eq!(
            HtItemNetId::from(ItemUidInput { slot: 7, serial: 9 }),
            HtItemNetId { solt: 7, serial: 9 }
        );
    }

    #[test]
    fn loadout_errors_keep_stable_codes() {
        assert_eq!(
            character_loadout_error(CharacterLoadoutError::InvalidJson).code,
            "empty_curtain_loadout_invalid"
        );
    }
}
