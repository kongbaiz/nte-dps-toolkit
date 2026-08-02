use std::path::Path;

pub(crate) use crate::core::character_data::CHARACTER_ATTRIBUTES;
use crate::core::character_data::{CharacterDataRecordInput, apply_character_data_record};
use crate::storage::resource::read_resource_text;

#[derive(Clone, Default)]
pub(crate) struct CharacterEditForm {
    pub(crate) id: String,
    pub(crate) name_zh: String,
    pub(crate) name_en: String,
    pub(crate) codename: String,
    pub(crate) attribute: String,
    pub(crate) verified: bool,
    pub(crate) color: String,
    pub(crate) avatar: String,
}

pub(crate) struct CharacterEditorState {
    pub(crate) document: serde_json::Value,
    pub(crate) selected_id: Option<String>,
    pub(crate) form: CharacterEditForm,
    pub(crate) search: String,
    pub(crate) new_id: String,
    pub(crate) dirty: bool,
    pub(crate) message: String,
    pub(crate) cancel_selection: Option<String>,
}

impl CharacterEditorState {
    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        let text = read_resource_text(path)
            .map_err(|error| format!("无法读取 {}: {error}", path.display()))?;
        let document: serde_json::Value =
            serde_json::from_str(&text).map_err(|error| format!("角色表 JSON 无效: {error}"))?;
        if !document
            .get("characters")
            .is_some_and(serde_json::Value::is_object)
        {
            return Err("characters.json 缺少 characters 对象".to_owned());
        }
        Ok(Self {
            document,
            selected_id: None,
            form: CharacterEditForm::default(),
            search: String::new(),
            new_id: String::new(),
            dirty: false,
            message: String::new(),
            cancel_selection: None,
        })
    }

    pub(crate) fn character_ids(&self) -> Vec<String> {
        let mut ids = self
            .document
            .get("characters")
            .and_then(serde_json::Value::as_object)
            .map(|characters| characters.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        ids.sort_by_key(|id| id.parse::<u32>().unwrap_or(u32::MAX));
        ids
    }

    pub(crate) fn select(&mut self, id: &str) {
        let Some(row) = self
            .document
            .get("characters")
            .and_then(serde_json::Value::as_object)
            .and_then(|characters| characters.get(id))
            .and_then(serde_json::Value::as_object)
        else {
            return;
        };
        self.selected_id = Some(id.to_owned());
        self.form = CharacterEditForm {
            id: id.to_owned(),
            name_zh: json_string_field(row, "name_zh"),
            name_en: json_string_field(row, "name_en"),
            codename: json_string_field(row, "codename"),
            attribute: json_string_field(row, "attribute"),
            verified: row
                .get("verified")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            color: json_string_field(row, "color"),
            avatar: json_string_field(row, "avatar"),
        };
        self.dirty = false;
        self.message.clear();
        self.cancel_selection = None;
    }

    pub(crate) fn start_new(&mut self) -> Result<(), String> {
        let id = self.new_id.trim();
        let parsed = id
            .parse::<u32>()
            .map_err(|_| "角色 ID 必须是正整数".to_owned())?;
        if parsed == 0 {
            return Err("角色 ID 必须大于 0".to_owned());
        }
        let id = parsed.to_string();
        if self
            .document
            .get("characters")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|characters| characters.contains_key(&id))
        {
            self.select(&id);
            return Err(format!("ID {id} 已存在，已切换到现有记录"));
        }
        self.cancel_selection = self.selected_id.clone();
        self.selected_id = None;
        self.form = CharacterEditForm {
            id,
            ..Default::default()
        };
        self.new_id.clear();
        self.dirty = true;
        self.message = "正在新增角色，填写后保存".to_owned();
        Ok(())
    }

    pub(crate) fn apply_form(&mut self) -> Result<String, String> {
        let record = apply_character_data_record(
            &mut self.document,
            CharacterDataRecordInput {
                original_id: self.selected_id.clone(),
                id: self.form.id.clone(),
                name_zh: self.form.name_zh.clone(),
                name_en: self.form.name_en.clone(),
                codename: self.form.codename.clone(),
                attribute: self.form.attribute.clone(),
                verified: self.form.verified,
                color: self.form.color.clone(),
                avatar: self.form.avatar.clone(),
            },
        )
        .map_err(|error| error.to_string())?;
        let id = record.id.to_string();
        self.selected_id = Some(id.clone());
        self.form.id = id.clone();
        self.dirty = false;
        self.cancel_selection = None;
        Ok(id)
    }

    pub(crate) fn cancel_edit(&mut self) {
        if let Some(id) = self
            .cancel_selection
            .take()
            .or_else(|| self.selected_id.clone())
        {
            self.select(&id);
        } else {
            self.form = CharacterEditForm::default();
            self.dirty = false;
            self.message.clear();
        }
    }
}

pub(crate) fn json_string_field(
    row: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> String {
    row.get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
