use serde::Deserialize;
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
}
