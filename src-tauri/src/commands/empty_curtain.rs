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
    platform::mods_plugin::{ModsPluginOperation, ModsPluginPlacement},
    storage::{i18n, io_util::atomic_write_text},
};
use serde::Deserialize;
use tauri::{State, WebviewWindow};
use ts_rs::TS;

use crate::{
    contract::{
        CommandError,
        empty_curtain::{
            EmptyCurtainFileResult, EmptyCurtainPositionSnapshot, EmptyCurtainSnapshot,
        },
    },
    equipment_operation_service::{EmptyCurtainOperationState, EquipmentOperationError},
    file_dialog::{self, DialogOutcome},
    state::{AppState, EmptyCurtainRuntimeError},
    windows::console,
};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ItemUidInput {
    slot: u32,
    serial: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EquipmentAction {
    Lock,
    Unlock,
    Discard,
    Restore,
    Unequip,
    Equip,
}

#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CharacterEquipmentAction {
    UnequipAll,
    OneClick,
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
pub(crate) async fn get_empty_curtain_snapshot(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<EmptyCurtainSnapshot, CommandError> {
    console::validate_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        refresh_plugin(&state)?;
        snapshot(&state)
    })
    .await
    .map_err(|_| CommandError::from(nte_dps_tool::platform::toolkit::ToolkitError::Failed))?
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
    let (items, characters, catalog) = state.empty_curtain_data_snapshot()?;
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
            CommandError::new(
                "empty_curtain_position_unavailable",
                "No compatible position is available for this drive module",
            )
        })?;
    Ok(positions
        .iter()
        .map(|position| EmptyCurtainPositionSnapshot {
            row: position.row,
            column: position.column,
        })
        .collect())
}

#[tauri::command]
pub(crate) async fn manage_empty_curtain_item(
    item: ItemUidInput,
    action: EquipmentAction,
    character: Option<ItemUidInput>,
    row: Option<i32>,
    column: Option<i32>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<EmptyCurtainSnapshot, CommandError> {
    console::validate_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        manage_empty_curtain_item_inner(item, action, character, row, column, &state)
    })
    .await
    .map_err(|_| CommandError::from(nte_dps_tool::platform::toolkit::ToolkitError::Failed))?
}

fn manage_empty_curtain_item_inner(
    item: ItemUidInput,
    action: EquipmentAction,
    character: Option<ItemUidInput>,
    row: Option<i32>,
    column: Option<i32>,
    state: &AppState,
) -> Result<EmptyCurtainSnapshot, CommandError> {
    let item_id = HtItemNetId::from(item);
    let (items, characters, catalog) = state.empty_curtain_data_snapshot()?;
    let operation = {
        let item = items
            .iter()
            .find(|item| item.id == item_id)
            .ok_or_else(item_unavailable)?;
        let definition = catalog
            .items
            .get(&item.item_id)
            .ok_or_else(metadata_unavailable)?;
        match action {
            EquipmentAction::Lock => Ok::<_, CommandError>((
                HtItemNetId::ZERO,
                ModsPluginOperation::SetItemLocked {
                    equipment: item.id,
                    locked: true,
                },
            )),
            EquipmentAction::Unlock => Ok((
                HtItemNetId::ZERO,
                ModsPluginOperation::SetItemLocked {
                    equipment: item.id,
                    locked: false,
                },
            )),
            EquipmentAction::Discard => Ok((
                HtItemNetId::ZERO,
                ModsPluginOperation::SetItemDiscarded {
                    equipment: item.id,
                    discarded: true,
                },
            )),
            EquipmentAction::Restore => Ok((
                HtItemNetId::ZERO,
                ModsPluginOperation::SetItemDiscarded {
                    equipment: item.id,
                    discarded: false,
                },
            )),
            EquipmentAction::Unequip => {
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
            EquipmentAction::Equip => {
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
        }
    }?;
    submit(state, operation.0, operation.1)?;
    snapshot(state)
}

#[tauri::command]
pub(crate) async fn apply_empty_curtain_character_action(
    character: ItemUidInput,
    action: CharacterEquipmentAction,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<EmptyCurtainSnapshot, CommandError> {
    console::validate_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        apply_empty_curtain_character_action_inner(character, action, &state)
    })
    .await
    .map_err(|_| CommandError::from(nte_dps_tool::platform::toolkit::ToolkitError::Failed))?
}

fn apply_empty_curtain_character_action_inner(
    character: ItemUidInput,
    action: CharacterEquipmentAction,
    state: &AppState,
) -> Result<EmptyCurtainSnapshot, CommandError> {
    let character_uid = HtItemNetId::from(character);
    let (items, characters, catalog) = state.empty_curtain_data_snapshot()?;
    let operation = {
        let character = characters
            .iter()
            .copied()
            .find(|candidate| candidate.net_id == character_uid)
            .ok_or_else(character_unavailable)?;
        match action {
            CharacterEquipmentAction::UnequipAll => Ok(ModsPluginOperation::UnequipAll),
            CharacterEquipmentAction::OneClick => recommended_loadout(character, &items, &catalog)
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
                .map_err(CommandError::from),
        }
    }?;
    submit(state, character_uid, operation)?;
    snapshot(state)
}

#[tauri::command]
pub(crate) async fn export_empty_curtain_inventory(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<EmptyCurtainFileResult, CommandError> {
    console::validate_window(&window)?;
    let (items, _, catalog) = state.empty_curtain_data_snapshot()?;
    if items.is_empty() {
        return Err(CommandError::new(
            "empty_curtain_inventory_empty",
            "No Console equipment to export",
        ));
    }
    let json = serde_json::to_string_pretty(&build_drive_calculator_inventory(&items, &catalog))
        .map_err(|error| {
            log::error!("serialize Drive Calculator inventory failed: {error}");
            CommandError::new(
                "empty_curtain_export_serialize_failed",
                "Failed to serialize Console equipment",
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
        snapshot: snapshot(state.inner())?,
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
    let (items, characters, catalog) = state.empty_curtain_data_snapshot()?;
    let character = characters
        .iter()
        .copied()
        .find(|candidate| candidate.net_id == character_uid)
        .ok_or_else(character_unavailable)?;
    let (json, character_id) = export_character_loadout_json(character, &items, &catalog)
        .map(|json| (json, character.character_id))?;
    let completed = save_json(
        &window,
        i18n::t("Console character loadout"),
        &format!("nte_loadout_{character_id}.json"),
        json,
    )
    .await?;
    Ok(EmptyCurtainFileResult {
        completed,
        snapshot: snapshot(state.inner())?,
    })
}

#[tauri::command]
pub(crate) async fn import_empty_curtain_loadout(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<EmptyCurtainFileResult, CommandError> {
    console::validate_window(&window)?;
    if !state.uses_plugin_equipment()? {
        return Err(super::toolkit::unsupported());
    }
    let Some(json) = open_json(&window, i18n::t("Console character loadout")).await? else {
        return Ok(EmptyCurtainFileResult {
            completed: false,
            snapshot: snapshot(state.inner())?,
        });
    };
    let (items, characters, catalog) = state.empty_curtain_data_snapshot()?;
    let file = parse_character_loadout_json(&json)?;
    let loadout = validate_character_loadout(&file, &characters, &items, &catalog)?;
    let operation = (
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
    );
    let owned = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || submit(&owned, operation.0, operation.1))
        .await
        .map_err(|_| CommandError::from(nte_dps_tool::platform::toolkit::ToolkitError::Failed))??;
    Ok(EmptyCurtainFileResult {
        completed: true,
        snapshot: snapshot(state.inner())?,
    })
}

pub(crate) fn snapshot(state: &AppState) -> Result<EmptyCurtainSnapshot, CommandError> {
    let operation = state.empty_curtain_operation_snapshot()?.operation;
    snapshot_with_operation(state, operation)
}

pub(crate) fn snapshot_with_operation(
    state: &AppState,
    operation: EmptyCurtainOperationState,
) -> Result<EmptyCurtainSnapshot, CommandError> {
    let inventory = state.empty_curtain_snapshot()?;
    let catalog = state.equipment_catalog();
    let resources = state.live_capture_resources();
    let can_operate = state.uses_plugin_equipment()?;
    let operation = if can_operate {
        operation
    } else {
        EmptyCurtainOperationState::default()
    };
    let mut snapshot =
        EmptyCurtainSnapshot::from_inventory(inventory, &catalog, &resources.characters, operation);
    snapshot.can_operate = can_operate;
    Ok(snapshot)
}

const CONFIRMING: &str =
    "Equipment operation is awaiting confirmation. Synchronizing inventory; do not repeat it.";
const CONFIRMATION_TIMEOUT: &str = "Equipment request was sent, but its result could not be confirmed. Check the game before retrying.";
pub(crate) fn settle_equipment_confirmation(
    state: &AppState,
    identity: &str,
    data: Option<&nte_dps_tool::core::user_equipment::Inventory>,
) -> Result<bool, CommandError> {
    use nte_dps_tool::core::equipment_runtime::ConfirmationState;
    let service = state.equipment_service();
    let outcome = service.inventory.settle_confirmation(identity, data)?;
    match outcome {
        Some(ConfirmationState::Waiting) => service.set("pending", CONFIRMING),
        Some(ConfirmationState::Confirmed) => service.set(
            "success",
            "Equipment change confirmed by refreshed inventory.",
        ),
        Some(ConfirmationState::Expired | ConfirmationState::SourceChanged) => {
            service.set("error", CONFIRMATION_TIMEOUT)
        }
        None => return Ok(false),
    }?;
    Ok(true)
}
impl From<nte_dps_tool::core::equipment_runtime::Error> for CommandError {
    fn from(error: nte_dps_tool::core::equipment_runtime::Error) -> Self {
        use nte_dps_tool::core::{equipment_runtime::Error, user_equipment};
        match error {
            Error::Transport(e) => {
                if e == nte_dps_tool::platform::toolkit::ToolkitError::SessionChanged {
                    return CommandError::new(
                        "equipment_snapshot_changed",
                        "Equipment data changed. Refresh before trying again.",
                    );
                }
                let mut error = CommandError::from(e);
                error.message_key = match e {
                    nte_dps_tool::platform::toolkit::ToolkitError::SessionChanged => {
                        "Equipment data changed. Refresh before trying again."
                    }
                    nte_dps_tool::platform::toolkit::ToolkitError::Busy => {
                        "Equipment synchronization is busy. Try again shortly."
                    }
                    nte_dps_tool::platform::toolkit::ToolkitError::InvalidProtocol
                    | nte_dps_tool::platform::toolkit::ToolkitError::Failed => {
                        "The equipment request could not be verified. Refresh and try again."
                    }
                    _ => {
                        "Equipment connection is unavailable. Check the User and Combat plugins, then refresh."
                    }
                };
                error
            }
            Error::ConfirmationPending(_) => {
                CommandError::new("equipment_confirmation_pending", CONFIRMING)
            }
            Error::Snapshot(user_equipment::Error::Changed) => CommandError::new(
                "equipment_snapshot_changed",
                "Equipment data changed. Refresh before trying again.",
            ),
            Error::Unconfirmed => CommandError::new(
                "equipment_unconfirmed",
                "Equipment request sent, but the refreshed inventory did not confirm the change.",
            ),
            Error::Snapshot(_) => CommandError::new(
                "equipment_snapshot_invalid",
                "A complete, verified equipment snapshot could not be read.",
            ),
        }
    }
}
fn equipment_permit(
    state: &AppState,
) -> Result<crate::state::PluginControlReservation, CommandError> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    loop {
        match state.reserve_plugin_control() {
            Ok(permit) => return Ok(permit),
            Err(e) if e.code == nte_dps_tool::core::CoreErrorCode::CaptureAlreadyRunning => {
                if std::time::Instant::now() >= deadline {
                    return Err(CommandError::new(
                        "equipment_sync_busy",
                        "Equipment synchronization is busy. Try again shortly.",
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(e) => return Err(CommandError::from(e)),
        }
    }
}
fn equipment_host() -> Result<nte_dps_tool::platform::toolkit::ToolkitClient, CommandError> {
    super::toolkit::client().map(|(host, _)| host).map_err(|mut e| {
        e.message_key =
            "Equipment connection is unavailable. Check the User and Combat plugins, then refresh.";
        e
    })
}
fn refresh_plugin(state: &AppState) -> Result<(), CommandError> {
    use nte_dps_tool::core::equipment_runtime;
    if !state.uses_plugin_equipment()? {
        return Ok(());
    }
    let _permit = equipment_permit(state)?;
    let host = equipment_host()?;
    let mut client =
        equipment_runtime::connect(&state.equipment_rpc(), &host, host.process_identity().0)?;
    let observed = equipment_runtime::watch(&host, &std::sync::atomic::AtomicBool::new(false))?;
    let data = equipment_runtime::collect(&mut client, &state.equipment_catalog())?;
    if !state.uses_plugin_equipment()? {
        return Err(super::toolkit::unsupported());
    }
    let settled = settle_equipment_confirmation(state, &host.identity(), Some(&data))?;
    state
        .equipment_service()
        .inventory
        .publish(host.identity(), data)?;
    state.equipment_service().inventory.set_watch(observed)?;
    if !settled {
        state
            .equipment_service()
            .set("idle", "No equipment operation is pending")?;
    }
    Ok(())
}
pub(crate) fn poll_plugin_changes(
    state: &AppState,
    stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), CommandError> {
    use nte_dps_tool::core::equipment_runtime;
    if stop.load(std::sync::atomic::Ordering::Acquire) || !state.uses_plugin_equipment()? {
        return Ok(());
    }
    let service = state.equipment_service();
    let (before, _) = service.inventory.get()?;
    let Some(before) = before else { return Ok(()) };
    let Ok(_permit) = state.reserve_plugin_control() else {
        return Ok(());
    };
    let (host, _) = super::toolkit::client().map_err(|mut e| {
        e.message_key =
            "Equipment connection is unavailable. Check the User and Combat plugins, then refresh.";
        e
    })?;
    let next = equipment_runtime::watch(&host, stop)?;
    let previous = service.inventory.watch()?;
    let confirming = service.inventory.confirming()?;
    if !confirming
        && service.inventory.identity()? == host.identity()
        && previous.as_ref().is_some_and(|p| {
            p.provider_id == next.provider_id && p.equipment_revision == next.equipment_revision
        })
    {
        if service.poll_snapshot()?.operation.message_key
            == "Equipment connection is unavailable. Check the User and Combat plugins, then refresh."
        {
            service.set("idle", "No equipment operation is pending")?;
        }
        return Ok(());
    }
    let mut client =
        equipment_runtime::connect(&state.equipment_rpc(), &host, host.process_identity().0)?
            .with_cancel(stop.clone());
    let data = if confirming {
        equipment_runtime::collect(&mut client, &state.equipment_catalog())?
    } else if let Some(previous) = previous
        .as_ref()
        .filter(|p| p.provider_id == next.provider_id)
    {
        match equipment_runtime::update_flags(&mut client, &before, previous, &next) {
            Ok(data) => data,
            Err(equipment_runtime::Error::Snapshot(_)) => {
                equipment_runtime::collect(&mut client, &state.equipment_catalog())?
            }
            Err(error) => return Err(CommandError::from(error)),
        }
    } else {
        equipment_runtime::collect(&mut client, &state.equipment_catalog())?
    };
    if stop.load(std::sync::atomic::Ordering::Acquire) {
        return Ok(());
    }
    let settled = settle_equipment_confirmation(state, &host.identity(), Some(&data))?;
    service.inventory.publish(host.identity(), data)?;
    service.inventory.set_watch(next)?;
    if !settled && service.poll_snapshot()?.operation.status == "error" {
        service.set("idle", "No equipment operation is pending")?;
    }
    Ok(())
}
fn submit(
    state: &AppState,
    character: HtItemNetId,
    operation: ModsPluginOperation,
) -> Result<(), CommandError> {
    use nte_dps_tool::core::equipment_runtime;
    if !state.uses_plugin_equipment()? {
        return Err(super::toolkit::unsupported());
    }
    let _permit = equipment_permit(state)?;
    let host = equipment_host()?;
    let service = state.equipment_service();
    if service.inventory.identity()? != host.identity() {
        return Err(CommandError::from(
            nte_dps_tool::platform::toolkit::ToolkitError::SessionChanged,
        ));
    }
    let (data, _) = service.inventory.get()?;
    let before = data.ok_or_else(item_unavailable)?;
    if service.inventory.confirming()? {
        return Err(CommandError::new(
            "equipment_confirmation_pending",
            CONFIRMING,
        ));
    }
    service.set("pending", "Sending equipment request...")?;
    let result = (|| {
        let mut client =
            equipment_runtime::connect(&state.equipment_rpc(), &host, host.process_identity().0)?;
        match equipment_runtime::execute(
            &mut client,
            &before,
            character,
            &operation,
            &state.equipment_catalog(),
        ) {
            Ok(after) => {
                service.inventory.publish(host.identity(), after)?;
                Ok(true)
            }
            Err(equipment_runtime::Error::ConfirmationPending(cause)) => {
                log::warn!("equipment confirmation readback deferred: {cause:?}");
                service.inventory.defer_confirmation(
                    host.identity(),
                    &before,
                    character,
                    operation.clone(),
                )?;
                Ok(false)
            }
            Err(error) => Err(CommandError::from(error)),
        }
    })();
    match &result {
        Ok(false) => service.set("pending", CONFIRMING),
        Ok(true) => service.set(
            "success",
            "Equipment change confirmed by refreshed inventory.",
        ),
        Err(e) => service.set("error", e.message_key),
    }?;
    result.map(|_| ())
}

impl From<EmptyCurtainRuntimeError> for CommandError {
    fn from(error: EmptyCurtainRuntimeError) -> Self {
        match error {
            EmptyCurtainRuntimeError::Capture(error) => CommandError::from(error),
            EmptyCurtainRuntimeError::Operation(error) => CommandError::from(error),
        }
    }
}

impl From<EquipmentOperationError> for CommandError {
    fn from(error: EquipmentOperationError) -> Self {
        match error {
            EquipmentOperationError::Unavailable => {
                CommandError::from(nte_dps_tool::platform::toolkit::ToolkitError::Unavailable)
            }
        }
    }
}

async fn save_json(
    window: &WebviewWindow,
    title: String,
    default_file_name: &str,
    json: String,
) -> Result<bool, CommandError> {
    #[cfg(windows)]
    {
        let default_file_name = default_file_name.to_owned();
        match file_dialog::choose_json_save_path(window, title, default_file_name)
            .await
            .map_err(|error| {
                log::error!("native Console equipment save dialog failed: {error}");
                file_dialog_error()
            })? {
            DialogOutcome::Selected(path) => tauri::async_runtime::spawn_blocking(move || {
                atomic_write_text(&path, &json)
                    .map(|_| true)
                    .map_err(|_| file_write_error())
            })
            .await
            .map_err(|_| file_dialog_error())?,
            DialogOutcome::Cancelled => Ok(false),
        }
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
        match file_dialog::choose_json_open_path(window, title)
            .await
            .map_err(|error| {
                log::error!("native Console equipment open dialog failed: {error}");
                file_dialog_error()
            })? {
            DialogOutcome::Selected(path) => tauri::async_runtime::spawn_blocking(move || {
                let metadata = fs::metadata(&path).map_err(|_| file_read_error())?;
                if metadata.len() > CHARACTER_LOADOUT_MAX_JSON_BYTES as u64 {
                    return Err(CommandError::from(CharacterLoadoutError::JsonTooLarge));
                }
                fs::read_to_string(path)
                    .map(Some)
                    .map_err(|_| file_read_error())
            })
            .await
            .map_err(|_| file_dialog_error())?,
            DialogOutcome::Cancelled => Ok(None),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (window, title);
        Err(file_dialog_error())
    }
}

impl From<CharacterLoadoutError> for CommandError {
    fn from(error: CharacterLoadoutError) -> Self {
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
        CommandError::with_arguments(code, key, arguments)
    }
}

impl From<RecommendedLoadoutError> for CommandError {
    fn from(error: RecommendedLoadoutError) -> Self {
        match error {
            RecommendedLoadoutError::MissingTemplate => CommandError::new(
                "empty_curtain_plan_unavailable",
                "No character equipment template is available",
            ),
            RecommendedLoadoutError::MissingEquipment => CommandError::new(
                "empty_curtain_plan_equipment_missing",
                "Required equipment for the character template is unavailable",
            ),
        }
    }
}

fn item_unavailable() -> CommandError {
    CommandError::new(
        "empty_curtain_item_unavailable",
        "Equipment is no longer present in the current inventory",
    )
}

fn character_unavailable() -> CommandError {
    CommandError::new(
        "empty_curtain_character_unavailable",
        "Character is no longer available in the current session",
    )
}

fn metadata_unavailable() -> CommandError {
    CommandError::new(
        "empty_curtain_metadata_unavailable",
        "Equipment metadata is unavailable",
    )
}

fn item_not_equipped() -> CommandError {
    CommandError::new(
        "empty_curtain_item_not_equipped",
        "Equipment is not currently equipped",
    )
}

fn position_required() -> CommandError {
    CommandError::new(
        "empty_curtain_position_required",
        "Choose a compatible drive module position",
    )
}

fn file_dialog_error() -> CommandError {
    CommandError::new(
        "empty_curtain_file_dialog_failed",
        "Console equipment file dialog failed",
    )
}

fn file_write_error() -> CommandError {
    CommandError::new(
        "empty_curtain_file_write_failed",
        "Failed to write Console equipment file",
    )
}

fn file_read_error() -> CommandError {
    CommandError::new(
        "empty_curtain_file_read_failed",
        "Failed to read character loadout",
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn capture_inventory_is_read_only_without_probing_the_host_or_replaying_old_errors() {
        let state = AppState::default();
        state.equipment_service().set("error",
            "Equipment connection is unavailable. Check the User and Combat plugins, then refresh.").unwrap();
        refresh_plugin(&state).unwrap();
        let view = snapshot(&state).unwrap();
        assert!(!view.can_operate);
        assert_eq!(view.operation.status, "idle");
        assert_eq!(
            state
                .equipment_service()
                .poll_snapshot()
                .unwrap()
                .operation
                .status,
            "error"
        );
        let result = submit(
            &state,
            HtItemNetId::ZERO,
            ModsPluginOperation::SetItemLocked {
                equipment: HtItemNetId::ZERO,
                locked: true,
            },
        );
        assert_eq!(result.unwrap_err().code, "plugin_unsupported");
    }

    #[test]
    fn snapshot_changes_and_post_dispatch_readbacks_are_not_connection_failures() {
        use nte_dps_tool::{
            core::equipment_runtime::{Error, ReadbackFailure},
            platform::toolkit::ToolkitError,
        };
        let changed = CommandError::from(Error::Transport(ToolkitError::SessionChanged));
        assert_eq!(changed.code, "equipment_snapshot_changed");
        let pending = CommandError::from(Error::ConfirmationPending(ReadbackFailure::Transport(
            ToolkitError::Busy,
        )));
        assert_eq!(pending.code, "equipment_confirmation_pending");
        assert_eq!(pending.message_key, super::CONFIRMING);
    }
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
            CommandError::from(CharacterLoadoutError::InvalidJson).code,
            "empty_curtain_loadout_invalid"
        );
    }

    #[test]
    fn retired_equipment_is_explicitly_unsupported() {
        let error = super::super::toolkit::unsupported();
        assert_eq!(error.code, "plugin_unsupported");
        assert!(error.message_arguments.is_empty());
        assert!(error.diagnostic_line.is_none());
    }
}
