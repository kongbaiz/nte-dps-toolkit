use serde::Serialize;

use nte_dps_tool::core::resource_audit::{
    ResourceAuditCategory, ResourceAuditCounts, ResourceAuditItem, ResourceAuditSeverity,
    ResourceAuditSummary,
};

pub(crate) const RESOURCES_CONTRACT_VERSION: u32 = 1;
pub(crate) const RESOURCES_DISPLAY_LIMIT: usize = 20_000;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResourcesSnapshot {
    pub contract_version: u32,
    pub error_count: usize,
    pub warning_count: usize,
    pub item_count: usize,
    pub display_limit: usize,
    pub counts: ResourceCountsSnapshot,
    pub items: Vec<ResourceItemSnapshot>,
    pub redacted_report: String,
}

impl ResourcesSnapshot {
    pub(crate) fn from_summary(summary: ResourceAuditSummary) -> Self {
        let error_count = summary.error_count();
        let warning_count = summary.warning_count();
        let item_count = summary.items.len();
        let redacted_report = summary.redacted_text();
        let ResourceAuditSummary { counts, items } = summary;
        Self {
            contract_version: RESOURCES_CONTRACT_VERSION,
            error_count,
            warning_count,
            item_count,
            display_limit: RESOURCES_DISPLAY_LIMIT,
            counts: counts.into(),
            items: items
                .into_iter()
                .take(RESOURCES_DISPLAY_LIMIT)
                .map(ResourceItemSnapshot::from)
                .collect(),
            redacted_report,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResourceCountsSnapshot {
    pub characters: usize,
    pub skill_damage: usize,
    pub mapped_effects: usize,
    pub semantic_effects: usize,
    pub abyss_monsters: usize,
    pub reactions: usize,
}

impl From<ResourceAuditCounts> for ResourceCountsSnapshot {
    fn from(counts: ResourceAuditCounts) -> Self {
        Self {
            characters: counts.characters,
            skill_damage: counts.skill_damage,
            mapped_effects: counts.mapped_effects,
            semantic_effects: counts.semantic_effects,
            abyss_monsters: counts.abyss_monsters,
            reactions: counts.reactions,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResourceItemSnapshot {
    pub severity: &'static str,
    pub category: &'static str,
    pub resource_id: String,
    pub display_name: String,
    pub message_key: String,
    pub message_arguments: Vec<String>,
    pub suggested_source: String,
}

impl From<ResourceAuditItem> for ResourceItemSnapshot {
    fn from(item: ResourceAuditItem) -> Self {
        let (message_key, message_arguments) = message_projection(&item.message);
        Self {
            severity: severity_code(item.severity),
            category: category_code(item.category),
            resource_id: item.resource_id,
            display_name: display_name_projection(item.display_name),
            message_key,
            message_arguments,
            suggested_source: item.suggested_source,
        }
    }
}

const fn severity_code(severity: ResourceAuditSeverity) -> &'static str {
    match severity {
        ResourceAuditSeverity::Error => "error",
        ResourceAuditSeverity::Warning => "warning",
    }
}

const fn category_code(category: ResourceAuditCategory) -> &'static str {
    match category {
        ResourceAuditCategory::Character => "character",
        ResourceAuditCategory::Skill => "skill",
        ResourceAuditCategory::GameplayEffect => "gameplayEffect",
        ResourceAuditCategory::Abyss => "abyss",
        ResourceAuditCategory::Reaction => "reaction",
        ResourceAuditCategory::File => "file",
    }
}

fn display_name_projection(display_name: String) -> String {
    match display_name.as_str() {
        "未命名角色" => "Unnamed character".to_owned(),
        "未配置反应" => "Unconfigured reaction".to_owned(),
        "反应文字" => "Reaction text".to_owned(),
        _ => display_name,
    }
}

fn message_projection(message: &str) -> (String, Vec<String>) {
    if message.starts_with("资源读取失败：") {
        return ("Resource file could not be read.".to_owned(), Vec::new());
    }
    if message.starts_with("JSON 无效：") {
        return ("Resource JSON is invalid.".to_owned(), Vec::new());
    }
    let key = match message {
        "缺少 characters 对象" => "Resource data is missing the characters object.",
        "缺少中文名" => "Character is missing a Chinese name.",
        "属性图标缺失" => "Character attribute icon is missing.",
        "属性值未识别" => "Character attribute is unrecognized.",
        "缺少属性" => "Character attribute is missing.",
        "头像资源缺失" => "Character avatar is missing.",
        "缺少头像路径" => "Character avatar path is missing.",
        "技能表存在但 GE index 映射缺失" => "Skill table entry has no GE index mapping.",
        "缺少技能分类或能力名" => "Skill is missing a category or ability name.",
        "GE 语义缺少技能表记录" => "GE semantics has no skill table record.",
        "GE 语义缺少 GE index 映射" => "GE semantics has no GE index mapping.",
        "深渊怪物头像缺失" => "Abyss monster portrait is missing.",
        "反应表缺少该 ID" => "Reaction table has no entry for this ID.",
        "反应文字素材缺失" => "Reaction text asset is missing.",
        _ => message,
    };
    (key.to_owned(), Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resources_contract_serializes_stable_codes_and_camel_case_counts() {
        let snapshot = ResourcesSnapshot::from_summary(ResourceAuditSummary {
            counts: ResourceAuditCounts {
                characters: 3,
                skill_damage: 4,
                mapped_effects: 5,
                semantic_effects: 6,
                abyss_monsters: 7,
                reactions: 8,
            },
            items: vec![ResourceAuditItem {
                severity: ResourceAuditSeverity::Warning,
                category: ResourceAuditCategory::GameplayEffect,
                resource_id: "GE_Test".to_owned(),
                display_name: "GE_Test".to_owned(),
                message: "技能表存在但 GE index 映射缺失".to_owned(),
                suggested_source: "res/data/ge.json".to_owned(),
            }],
        });

        let value = serde_json::to_value(snapshot).expect("resources snapshot must serialize");
        assert_eq!(value["contractVersion"], RESOURCES_CONTRACT_VERSION);
        assert_eq!(value["itemCount"], 1);
        assert_eq!(value["displayLimit"], RESOURCES_DISPLAY_LIMIT);
        assert_eq!(value["counts"]["skillDamage"], 4);
        assert_eq!(value["items"][0]["severity"], "warning");
        assert_eq!(value["items"][0]["category"], "gameplayEffect");
        assert_eq!(
            value["items"][0]["messageKey"],
            "Skill table entry has no GE index mapping."
        );
        assert!(value.get("contract_version").is_none());
    }

    #[test]
    fn resources_contract_separates_dynamic_file_error_arguments() {
        let (key, arguments) = message_projection("资源读取失败：access denied");
        assert_eq!(key, "Resource file could not be read.");
        assert!(arguments.is_empty());
    }
}
