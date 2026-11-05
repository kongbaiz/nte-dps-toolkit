//! Release training commands (415/416). Provider and target identities come from the same snapshot.
use crate::{contract::CommandError, state::AppState, windows::console};
use nte_dps_tool::{core::toolkit::DataMode, platform::toolkit::ToolkitError};
use serde::{Deserialize, Serialize};
use tauri::{State, WebviewWindow};
use ts_rs::TS;

#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
pub(crate) struct TrainingEnemies {
    provider: String,
    templates: Vec<String>,
    attributes: Vec<String>,
    targets: Vec<TrainingTarget>,
    message: String,
    error: String,
}

#[derive(Deserialize, Serialize, TS)]
#[ts(export)]
struct TrainingTarget {
    index: u32,
    serial: u32,
    name: String,
    values: Vec<Option<f32>>,
}

#[derive(Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum EnemyAction {
    Spawn {
        template: u32,
        presets: Vec<(u32, f32)>,
    },
    Custom {
        blueprint: String,
        level: u32,
        pack: String,
        presets: Vec<(u32, f32)>,
    },
    Read {
        index: u32,
        serial: u32,
    },
    Edit {
        index: u32,
        serial: u32,
        attribute: u32,
        value: f32,
    },
    Remove {
        index: u32,
        serial: u32,
    },
}

#[derive(Deserialize, TS)]
#[ts(export)]
pub(crate) struct EnemyRequest {
    provider: String,
    action: EnemyAction,
}
impl EnemyRequest {
    fn wire(self) -> Result<(u32, String), ToolkitError> {
        // The native request is whitespace-delimited. A string field must remain one token.
        let token = |s: &str| !s.is_empty() && !s.chars().any(char::is_whitespace);
        if !token(&self.provider) {
            return Err(ToolkitError::InvalidProtocol);
        }
        let suffix = |presets: Vec<(u32, f32)>| {
            presets
                .into_iter()
                .map(|(i, v)| format!(" {i} {v}"))
                .collect::<String>()
        };
        let (argument, fields) = match self.action {
            EnemyAction::Spawn { template, presets } => {
                (1, format!("{template}{}", suffix(presets)))
            }
            EnemyAction::Custom {
                blueprint,
                level,
                pack,
                presets,
            } if token(&blueprint) && token(&pack) => {
                (7, format!("{blueprint} {level} {pack}{}", suffix(presets)))
            }
            EnemyAction::Custom { .. } => return Err(ToolkitError::InvalidProtocol),
            EnemyAction::Read { index, serial } => (2, format!("{index} {serial}")),
            EnemyAction::Edit {
                index,
                serial,
                attribute,
                value,
            } => (3, format!("{index} {serial} {attribute} {value}")),
            EnemyAction::Remove { index, serial } => (4, format!("{index} {serial}")),
        };
        Ok((argument, format!("{} {fields}", self.provider)))
    }
}

#[tauri::command]
pub(crate) async fn training_enemies(
    expected_identity: String,
    request: Option<EnemyRequest>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<Option<TrainingEnemies>, CommandError> {
    console::validate_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = state.reserve_plugin_control()?;
        if state.data_mode() != DataMode::Plugin {
            return Err(super::toolkit::unsupported());
        }
        let (host, description) = super::toolkit::client()?;
        if description.operation_session != expected_identity {
            return Err(ToolkitError::SessionChanged.into());
        }
        let command = if request.is_some() { 416 } else { 415 };
        if !description.commands.contains(&command) {
            return Err(super::toolkit::unsupported());
        }
        if let Some(request) = request {
            let (argument, text) = request.wire()?;
            host.call(command, argument, &text, &|| false)?;
            Ok(None)
        } else {
            Ok(Some(host.json(command, 0, "", &|| false)?))
        }
    })
    .await
    .map_err(|_| CommandError::from(ToolkitError::Failed))?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enemy_edit_keeps_provider_and_target_serial_in_wire_order() {
        let action = EnemyAction::Edit {
            index: 19,
            serial: 32,
            attribute: 4,
            value: -0.25,
        };
        let request = EnemyRequest {
            provider: "training-a".into(),
            action,
        };
        assert_eq!(
            request.wire().unwrap(),
            (3, "training-a 19 32 4 -0.25".into())
        );
        let request = EnemyRequest {
            provider: "training-a".into(),
            action: EnemyAction::Custom {
                blueprint: "/Game/M.M_C".into(),
                level: 1,
                pack: "a 4 1".into(),
                presets: vec![],
            },
        };
        assert!(request.wire().is_err());
    }
}
