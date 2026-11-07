//! JSON Forms documents supplied by the currently loaded native plugin.
use super::{PAGE_BYTES, ToolkitClient, ToolkitDescription, ToolkitError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

#[derive(Deserialize, Serialize)]
#[cfg_attr(feature = "tauri_frontend", derive(ts_rs::TS))]
#[cfg_attr(feature = "tauri_frontend", ts(export))]
pub struct PluginForm {
    pub version: u32,
    pub title: String,
    #[cfg_attr(feature = "tauri_frontend", ts(type = "unknown"))]
    pub schema: Value,
    #[cfg_attr(feature = "tauri_frontend", ts(type = "unknown"))]
    pub uischema: Value,
    #[cfg_attr(feature = "tauri_frontend", ts(type = "unknown"))]
    pub data: Value,
    #[serde(default)]
    pub translations: BTreeMap<String, BTreeMap<String, String>>,
    pub actions: Vec<PluginFormAction>,
}

#[derive(Deserialize, Serialize)]
#[cfg_attr(feature = "tauri_frontend", derive(ts_rs::TS))]
#[cfg_attr(feature = "tauri_frontend", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PluginFormAction {
    pub id: String,
    pub label: String,
    pub confirmation: Option<String>,
    pub requires_idle: bool,
    #[serde(default)]
    pub dev: bool,
    #[serde(skip_serializing)]
    #[cfg_attr(feature = "tauri_frontend", ts(skip))]
    pub command: u32,
    #[serde(skip_serializing)]
    #[cfg_attr(feature = "tauri_frontend", ts(skip))]
    argument: Argument,
    #[serde(skip_serializing)]
    #[cfg_attr(feature = "tauri_frontend", ts(skip))]
    text: Vec<TextPart>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Argument {
    Constant { value: u32 },
    Field { path: String },
    Flags { fields: BTreeMap<String, u32> },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum TextPart {
    Literal { value: String },
    Field { path: String },
    Repeat { path: String, fields: Vec<String> },
}

#[derive(Serialize)]
#[cfg_attr(feature = "tauri_frontend", derive(ts_rs::TS))]
#[cfg_attr(feature = "tauri_frontend", ts(export))]
pub struct PluginFormSnapshot {
    pub owner: String,
    pub form: PluginForm,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativePlugin {
    file: String,
    state: String,
    service_owner: Option<u64>,
}

pub fn read(
    client: &ToolkitClient,
    description: &ToolkitDescription,
    file: &str,
) -> Result<PluginFormSnapshot, ToolkitError> {
    if !description.commands.contains(&12) {
        return Err(ToolkitError::Unsupported);
    }
    let plugins: Vec<NativePlugin> = client.json(3, 0, "", &|| false)?;
    let owner = plugins
        .into_iter()
        .find(|plugin| plugin.file == file && plugin.state == "loaded")
        .and_then(|plugin| plugin.service_owner)
        .ok_or(ToolkitError::OperationUnavailable)?;
    let mut services = description.services.iter().filter(|service| {
        service.owner == owner
            && service.name.starts_with("ui.jsonforms.v1.")
            && service.purpose == "snapshot"
    });
    let service = services.next().ok_or(ToolkitError::Unsupported)?;
    if services.next().is_some() {
        return Err(ToolkitError::InvalidProtocol);
    }
    let bytes = client.call_owned(
        &description.operation_session,
        owner,
        service.command,
        0,
        "",
        &|| false,
    )?;
    let form = parse(&bytes, description, owner)?;
    Ok(PluginFormSnapshot {
        owner: owner.to_string(),
        form,
    })
}

fn parse(
    bytes: &[u8],
    description: &ToolkitDescription,
    owner: u64,
) -> Result<PluginForm, ToolkitError> {
    if bytes.len() > PAGE_BYTES {
        return Err(ToolkitError::TooLarge);
    }
    let form: PluginForm =
        serde_json::from_slice(bytes).map_err(|_| ToolkitError::InvalidProtocol)?;
    if form.version != 1 {
        return Err(ToolkitError::Unsupported);
    }
    if form.title.is_empty()
        || !form.schema.is_object()
        || !form.uischema.is_object()
        || !form.data.is_object()
    {
        return Err(ToolkitError::InvalidProtocol);
    }
    let mut ids = HashSet::new();
    for action in &form.actions {
        if action.id.is_empty()
            || !ids.insert(&action.id)
            || !description.services.iter().any(|service| {
                service.owner == owner
                    && service.command == action.command
                    && service.command >= 100
                    && service.command != 200
            })
        {
            return Err(ToolkitError::InvalidProtocol);
        }
    }
    Ok(form)
}

fn value<'a>(data: &'a Value, path: &str) -> Result<&'a Value, ToolkitError> {
    data.pointer(path).ok_or(ToolkitError::InvalidProtocol)
}

fn token(value: &Value) -> Result<String, ToolkitError> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        Value::Bool(enabled) => Ok(u8::from(*enabled).to_string()),
        _ => Err(ToolkitError::InvalidProtocol),
    }
}

impl PluginFormAction {
    pub fn invoke(
        &self,
        client: &ToolkitClient,
        session: &str,
        owner: &str,
        data: &Value,
    ) -> Result<Vec<u8>, ToolkitError> {
        if self.dev && !cfg!(debug_assertions) {
            return Err(ToolkitError::Unsupported);
        }
        let (argument, text) = self.wire(data)?;
        let owner = owner.parse().map_err(|_| ToolkitError::InvalidProtocol)?;
        client.call_owned(session, owner, self.command, argument, &text, &|| false)
    }

    fn wire(&self, data: &Value) -> Result<(u32, String), ToolkitError> {
        let argument = match &self.argument {
            Argument::Constant { value } => *value,
            Argument::Field { path } => {
                let number = match value(data, path)? {
                    Value::Bool(enabled) => u64::from(*enabled),
                    value => value.as_u64().ok_or(ToolkitError::InvalidProtocol)?,
                };
                number
                    .try_into()
                    .map_err(|_| ToolkitError::InvalidProtocol)?
            }
            Argument::Flags { fields } => {
                let mut bits = 0;
                for (path, mask) in fields {
                    if value(data, path)?
                        .as_bool()
                        .ok_or(ToolkitError::InvalidProtocol)?
                    {
                        bits |= mask;
                    }
                }
                bits
            }
        };
        let mut parts = Vec::new();
        for part in &self.text {
            match part {
                TextPart::Literal { value } => parts.push(value.clone()),
                TextPart::Field { path } => parts.push(token(value(data, path)?)?),
                TextPart::Repeat { path, fields } => {
                    for row in value(data, path)?
                        .as_array()
                        .ok_or(ToolkitError::InvalidProtocol)?
                    {
                        for field in fields {
                            parts.push(token(value(row, field)?)?);
                        }
                    }
                }
            }
        }
        Ok((argument, parts.join(" ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plugin_form_binds_actions_to_owner_and_encodes_form_values() {
        let action = json!({"id":"edit", "label":"Edit", "command":416,
            "requiresIdle":false, "argument":{"kind":"field","path":"/operation"},
            "text":[{"kind":"field","path":"/provider"},
                {"kind":"repeat","path":"/presets","fields":["/attribute","/value"]}]});
        let mut doc = json!({"version":1,"title":"Unknown plugin","schema":{"type":"object"},
            "uischema":{"type":"VerticalLayout","elements":[]},"data":{},"actions":[action]});
        let description = ToolkitDescription {
            operation_session: String::new(),
            commands: vec![12, 416],
            services: vec![super::super::ToolkitService {
                name: "custom.edit".into(),
                owner: 7,
                command: 416,
                purpose: "command".into(),
            }],
        };
        let form = parse(&serde_json::to_vec(&doc).unwrap(), &description, 7).unwrap();
        assert_eq!(
            form.actions[0]
                .wire(
                    &json!({"operation":3,"provider":"p1","presets":[{"attribute":4,"value":0.25}]})
                )
                .unwrap(),
            (3, "p1 4 0.25".into())
        );
        assert!(form.actions[0].wire(&json!({"operation":-1})).is_err());
        assert!(parse(&serde_json::to_vec(&doc).unwrap(), &description, 8).is_err());
        doc["actions"][0]["argument"] = json!({"kind":"flags","fields":{"/a":1,"/b":8}});
        doc["actions"][0]["text"] = json!([]);
        let form = parse(&serde_json::to_vec(&doc).unwrap(), &description, 7).unwrap();
        assert_eq!(
            form.actions[0].wire(&json!({"a":true,"b":true})).unwrap(),
            (9, String::new())
        );
        assert!(parse(b"{}", &description, 7).is_err());
    }
}
