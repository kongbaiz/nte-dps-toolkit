use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::jsonrpc::RpcError;

#[derive(Debug)]
pub enum Request {
    Hello(HelloParams),
    Status,
    Shutdown,
    CaptureDetect,
    CaptureStart(CaptureStartParams),
    CaptureStop,
    InventoryGetLatest,
    Equipment(EquipmentOperationParam),
    BattleGetSummary(BattleSummaryParams),
    BattleReset,
    Unknown,
}

#[derive(Debug, Deserialize)]
pub struct HelloParams {
    pub client_name: String,
    pub client_version: String,
    pub protocol_min: u32,
    pub protocol_max: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureProfileParam {
    Inventory,
    Combat,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum CaptureDeviceParam {
    Auto,
    Name { name: String },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RawCaptureParam {
    Enabled,
    Disabled,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CaptureStartParams {
    pub profile: CaptureProfileParam,
    pub device: CaptureDeviceParam,
    pub include_incoming: bool,
    pub server_damage_calibration: bool,
    pub raw_capture: Option<RawCaptureParam>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub struct BattleSummaryParams {
    pub subtract_time_stop: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub struct ItemUidParam {
    pub slot: u32,
    pub serial: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub struct EquipmentPlacementParam {
    pub equipment: ItemUidParam,
    pub row: i32,
    pub column: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EquipmentOperationParam {
    EquipModule {
        character: ItemUidParam,
        equipment: ItemUidParam,
        row: i32,
        column: i32,
    },
    EquipCore {
        character: ItemUidParam,
        equipment: ItemUidParam,
    },
    UnequipModule {
        character: ItemUidParam,
        equipment: ItemUidParam,
    },
    UnequipCore {
        character: ItemUidParam,
        equipment: ItemUidParam,
    },
    UnequipAll {
        character: ItemUidParam,
    },
    EquipOneKey {
        character: ItemUidParam,
        placements: Vec<EquipmentPlacementParam>,
        core: ItemUidParam,
    },
    MoveModuleToCharacter {
        character: ItemUidParam,
        equipment: ItemUidParam,
        row: i32,
        column: i32,
    },
    MoveCoreToCharacter {
        character: ItemUidParam,
        equipment: ItemUidParam,
    },
    SetItemDiscarded {
        equipment: ItemUidParam,
        discarded: bool,
    },
    SetItemLocked {
        equipment: ItemUidParam,
        locked: bool,
    },
}

#[derive(Debug, Deserialize)]
struct ModuleParams {
    character: ItemUidParam,
    equipment: ItemUidParam,
    row: i32,
    column: i32,
}

#[derive(Debug, Deserialize)]
struct CharacterEquipmentParams {
    character: ItemUidParam,
    equipment: ItemUidParam,
}

#[derive(Debug, Deserialize)]
struct CharacterParams {
    character: ItemUidParam,
}

#[derive(Debug, Deserialize)]
struct OneKeyParams {
    character: ItemUidParam,
    placements: Vec<EquipmentPlacementParam>,
    core: ItemUidParam,
}

#[derive(Debug, Deserialize)]
struct SetDiscardedParams {
    equipment: ItemUidParam,
    discarded: bool,
}

#[derive(Debug, Deserialize)]
struct SetLockedParams {
    equipment: ItemUidParam,
    locked: bool,
}

pub fn parse_request(method: &str, params: Value) -> Result<Request, RpcError> {
    match method {
        "core.hello" => {
            let params: HelloParams = serde_json::from_value(params).map_err(|_| {
                RpcError::invalid_params("core.hello requires all handshake fields")
            })?;
            if params.client_name.is_empty() || params.client_version.is_empty() {
                return Err(RpcError::invalid_params(
                    "client_name and client_version must not be empty",
                ));
            }
            if params.protocol_min > params.protocol_max {
                return Err(RpcError::invalid_params(
                    "protocol_min must not exceed protocol_max",
                ));
            }
            Ok(Request::Hello(params))
        }
        "core.status" => {
            validate_empty_params(&params)?;
            Ok(Request::Status)
        }
        "core.shutdown" => {
            validate_empty_params(&params)?;
            Ok(Request::Shutdown)
        }
        "capture.detect" => {
            validate_empty_params(&params)?;
            Ok(Request::CaptureDetect)
        }
        "capture.start" => {
            let params: CaptureStartParams = serde_json::from_value(params)
                .map_err(|_| RpcError::invalid_params("capture.start parameters are invalid"))?;
            if matches!(&params.device, CaptureDeviceParam::Name { name } if name.trim().is_empty())
            {
                return Err(RpcError::invalid_params("device name must not be empty"));
            }
            Ok(Request::CaptureStart(params))
        }
        "capture.stop" => {
            validate_empty_params(&params)?;
            Ok(Request::CaptureStop)
        }
        "inventory.get_latest" => {
            validate_empty_params(&params)?;
            Ok(Request::InventoryGetLatest)
        }
        "equipment.equip_module" => {
            let params: ModuleParams = parse_params(params, method)?;
            validate_uid(params.character, "character")?;
            validate_uid(params.equipment, "equipment")?;
            validate_grid_position(params.row, params.column)?;
            Ok(Request::Equipment(EquipmentOperationParam::EquipModule {
                character: params.character,
                equipment: params.equipment,
                row: params.row,
                column: params.column,
            }))
        }
        "equipment.equip_core" => {
            let params: CharacterEquipmentParams = parse_params(params, method)?;
            validate_uid(params.character, "character")?;
            validate_uid(params.equipment, "equipment")?;
            Ok(Request::Equipment(EquipmentOperationParam::EquipCore {
                character: params.character,
                equipment: params.equipment,
            }))
        }
        "equipment.unequip_module" => {
            let params: CharacterEquipmentParams = parse_params(params, method)?;
            validate_uid(params.character, "character")?;
            validate_uid(params.equipment, "equipment")?;
            Ok(Request::Equipment(EquipmentOperationParam::UnequipModule {
                character: params.character,
                equipment: params.equipment,
            }))
        }
        "equipment.unequip_core" => {
            let params: CharacterEquipmentParams = parse_params(params, method)?;
            validate_uid(params.character, "character")?;
            validate_uid(params.equipment, "equipment")?;
            Ok(Request::Equipment(EquipmentOperationParam::UnequipCore {
                character: params.character,
                equipment: params.equipment,
            }))
        }
        "equipment.unequip_all" => {
            let params: CharacterParams = parse_params(params, method)?;
            validate_uid(params.character, "character")?;
            Ok(Request::Equipment(EquipmentOperationParam::UnequipAll {
                character: params.character,
            }))
        }
        "equipment.equip_one_key" => {
            let params: OneKeyParams = parse_params(params, method)?;
            validate_uid(params.character, "character")?;
            validate_uid(params.core, "core")?;
            if params.placements.is_empty() || params.placements.len() > 64 {
                return Err(RpcError::invalid_params(
                    "placements must contain between 1 and 64 entries",
                ));
            }
            for placement in &params.placements {
                validate_uid(placement.equipment, "placement equipment")?;
                validate_grid_position(placement.row, placement.column)?;
            }
            Ok(Request::Equipment(EquipmentOperationParam::EquipOneKey {
                character: params.character,
                placements: params.placements,
                core: params.core,
            }))
        }
        "equipment.move_module_to_character" => {
            let params: ModuleParams = parse_params(params, method)?;
            validate_uid(params.character, "character")?;
            validate_uid(params.equipment, "equipment")?;
            validate_grid_position(params.row, params.column)?;
            Ok(Request::Equipment(
                EquipmentOperationParam::MoveModuleToCharacter {
                    character: params.character,
                    equipment: params.equipment,
                    row: params.row,
                    column: params.column,
                },
            ))
        }
        "equipment.move_core_to_character" => {
            let params: CharacterEquipmentParams = parse_params(params, method)?;
            validate_uid(params.character, "character")?;
            validate_uid(params.equipment, "equipment")?;
            Ok(Request::Equipment(
                EquipmentOperationParam::MoveCoreToCharacter {
                    character: params.character,
                    equipment: params.equipment,
                },
            ))
        }
        "equipment.set_item_discarded" => {
            let params: SetDiscardedParams = parse_params(params, method)?;
            validate_uid(params.equipment, "equipment")?;
            Ok(Request::Equipment(
                EquipmentOperationParam::SetItemDiscarded {
                    equipment: params.equipment,
                    discarded: params.discarded,
                },
            ))
        }
        "equipment.set_item_locked" => {
            let params: SetLockedParams = parse_params(params, method)?;
            validate_uid(params.equipment, "equipment")?;
            Ok(Request::Equipment(EquipmentOperationParam::SetItemLocked {
                equipment: params.equipment,
                locked: params.locked,
            }))
        }
        "battle.get_summary" => {
            let params: BattleSummaryParams = serde_json::from_value(params).map_err(|_| {
                RpcError::invalid_params("battle.get_summary requires subtract_time_stop")
            })?;
            Ok(Request::BattleGetSummary(params))
        }
        "battle.reset" => {
            validate_empty_params(&params)?;
            Ok(Request::BattleReset)
        }
        _ => Ok(Request::Unknown),
    }
}

fn parse_params<T: DeserializeOwned>(params: Value, method: &str) -> Result<T, RpcError> {
    serde_json::from_value(params)
        .map_err(|_| RpcError::invalid_params(format!("{method} parameters are invalid")))
}

fn validate_uid(uid: ItemUidParam, field: &str) -> Result<(), RpcError> {
    if uid.slot == 0 || uid.serial == 0 || uid.slot == u32::MAX || uid.serial == u32::MAX {
        Err(RpcError::invalid_params(format!(
            "{field} slot and serial must be nonzero and must not be 4294967295"
        )))
    } else {
        Ok(())
    }
}

fn validate_grid_position(row: i32, column: i32) -> Result<(), RpcError> {
    if (1..=5).contains(&row) && (1..=5).contains(&column) {
        Ok(())
    } else {
        Err(RpcError::invalid_params(
            "row and column must both be in the range 1..5",
        ))
    }
}

fn validate_empty_params(params: &Value) -> Result<(), RpcError> {
    if params.is_null() || params.is_object() {
        Ok(())
    } else {
        Err(RpcError::invalid_params(
            "params must be an object when provided",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_hello_protocol_range() {
        let error = parse_request(
            "core.hello",
            serde_json::json!({
                "client_name": "client",
                "client_version": "1",
                "protocol_min": 2,
                "protocol_max": 1,
            }),
        )
        .unwrap_err();
        assert_eq!(error.data.unwrap().domain_code, "INVALID_PARAMS");
    }

    #[test]
    fn no_param_methods_reject_arrays() {
        assert!(parse_request("core.status", serde_json::json!([])).is_err());
    }

    #[test]
    fn capture_start_accepts_explicitly_disabled_raw_capture() {
        let request = parse_request(
            "capture.start",
            serde_json::json!({
                "profile": "inventory",
                "device": {"mode": "auto"},
                "include_incoming": true,
                "server_damage_calibration": true,
                "raw_capture": "disabled"
            }),
        )
        .unwrap();
        let Request::CaptureStart(params) = request else {
            panic!("capture.start must produce typed parameters");
        };
        assert_eq!(params.profile, CaptureProfileParam::Inventory);
        assert_eq!(params.device, CaptureDeviceParam::Auto);
        assert_eq!(params.raw_capture, Some(RawCaptureParam::Disabled));
    }

    #[test]
    fn capture_start_rejects_unknown_profiles_and_blank_manual_devices() {
        let base = serde_json::json!({
            "profile": "other",
            "device": {"mode": "auto"},
            "include_incoming": true,
            "server_damage_calibration": true
        });
        assert!(parse_request("capture.start", base).is_err());
        assert!(
            parse_request(
                "capture.start",
                serde_json::json!({
                    "profile": "combat",
                    "device": {"mode": "name", "name": ""},
                    "include_incoming": false,
                    "server_damage_calibration": false
                })
            )
            .is_err()
        );
    }

    #[test]
    fn battle_summary_requires_a_boolean_time_stop_mode() {
        let request = parse_request(
            "battle.get_summary",
            serde_json::json!({"subtract_time_stop": true}),
        )
        .unwrap();
        let Request::BattleGetSummary(params) = request else {
            panic!("battle.get_summary must produce typed parameters");
        };
        assert!(params.subtract_time_stop);
        assert!(parse_request("battle.get_summary", serde_json::json!({})).is_err());
        assert!(
            parse_request(
                "battle.get_summary",
                serde_json::json!({"subtract_time_stop": "yes"})
            )
            .is_err()
        );
    }

    #[test]
    fn equipment_methods_validate_uids_positions_and_boolean_state() {
        let request = parse_request(
            "equipment.move_module_to_character",
            serde_json::json!({
                "character": {"slot": 1, "serial": 2},
                "equipment": {"slot": 3, "serial": 4},
                "row": 2,
                "column": 5
            }),
        )
        .unwrap();
        assert!(matches!(
            request,
            Request::Equipment(EquipmentOperationParam::MoveModuleToCharacter {
                row: 2,
                column: 5,
                ..
            })
        ));
        assert!(
            parse_request(
                "equipment.equip_module",
                serde_json::json!({
                    "character": {"slot": 0, "serial": 0},
                    "equipment": {"slot": 3, "serial": 4},
                    "row": 1,
                    "column": 1
                })
            )
            .is_err()
        );
        assert!(validate_uid(ItemUidParam { slot: 0, serial: 1 }, "equipment").is_err());
        assert!(validate_uid(ItemUidParam { slot: 1, serial: 0 }, "equipment").is_err());
        assert!(
            validate_uid(
                ItemUidParam {
                    slot: u32::MAX,
                    serial: 1,
                },
                "equipment"
            )
            .is_err()
        );
        assert!(
            validate_uid(
                ItemUidParam {
                    slot: 1,
                    serial: u32::MAX,
                },
                "equipment"
            )
            .is_err()
        );
        assert!(validate_uid(ItemUidParam { slot: 1, serial: 2 }, "equipment").is_ok());
        assert!(
            parse_request(
                "equipment.move_module_to_character",
                serde_json::json!({
                    "character": {"slot": 1, "serial": 2},
                    "equipment": {"slot": 3, "serial": 4},
                    "row": 0,
                    "column": 6
                })
            )
            .is_err()
        );
        assert!(
            parse_request(
                "equipment.set_item_locked",
                serde_json::json!({
                    "equipment": {"slot": 3, "serial": 4},
                    "locked": 1
                })
            )
            .is_err()
        );
    }

    #[test]
    fn one_key_equipment_requires_a_bounded_nonempty_plan() {
        let base = serde_json::json!({
            "character": {"slot": 1, "serial": 2},
            "placements": [{
                "equipment": {"slot": 3, "serial": 4},
                "row": 1,
                "column": 2
            }],
            "core": {"slot": 5, "serial": 6}
        });
        assert!(parse_request("equipment.equip_one_key", base).is_ok());
        assert!(
            parse_request(
                "equipment.equip_one_key",
                serde_json::json!({
                    "character": {"slot": 1, "serial": 2},
                    "placements": [],
                    "core": {"slot": 5, "serial": 6}
                })
            )
            .is_err()
        );
    }
}
