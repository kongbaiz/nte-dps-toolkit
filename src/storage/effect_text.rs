//! Resource-backed display projection; never changes the frozen combat observation.
use crate::{
    engine::{
        model::plugin_snapshot::{Effect, PluginHitSnapshot},
        rich_text::strip_tags,
    },
    storage::{
        i18n::{Language, t_for, tf_for},
        resource::read_resource_text,
    },
};
use serde::Deserialize;
use std::{collections::HashMap, path::Path, sync::LazyLock};

#[derive(Clone, Copy, Debug)]
pub enum EffectTextError {
    Resource,
    Format,
}
#[derive(Deserialize)]
struct Catalog {
    entries: HashMap<String, Entry>,
}
#[derive(Deserialize)]
struct Entry {
    name: String,
    description: String,
    origin: Origin,
    parameters: Vec<Option<f64>>,
}
#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Origin {
    Buff,
    Ability,
    Awakening,
    Arc,
    Equipment,
    Tag,
}
static CATALOG: LazyLock<Result<Catalog, EffectTextError>> = LazyLock::new(|| {
    let text = read_resource_text(Path::new("res/data/skills/effect_texts.json"))
        .map_err(|_| EffectTextError::Resource)?;
    let catalog: Catalog = serde_json::from_str(&text).map_err(|_| EffectTextError::Format)?;
    Ok(catalog)
});

fn key(value: &str) -> String {
    let key = value.to_ascii_lowercase();
    key.strip_suffix("_c").unwrap_or(&key).to_owned()
}

fn format_description(
    template: &str,
    parameters: &[Option<f64>],
    has_recorded_modifiers: bool,
    language: Language,
) -> String {
    let mut rest = template;
    let mut output = String::new();
    let mut missing = false;
    let mut filled = false;
    while let Some((before, tail)) = rest.split_once('{') {
        output.push_str(before);
        let Some((index, after)) = tail.split_once('}') else {
            output.push('{');
            output.push_str(tail);
            rest = "";
            break;
        };
        match index.parse::<usize>() {
            Ok(index) => {
                if let Some(Some(parameter)) = parameters.get(index) {
                    let number = format!("{parameter:.4}");
                    output.push_str(number.trim_end_matches('0').trim_end_matches('.'));
                    filled = true;
                } else {
                    output.push('[');
                    output.push_str(&t_for(language, "Value not recorded"));
                    output.push(']');
                    missing = true;
                }
            }
            Err(_) => {
                output.push('{');
                output.push_str(index);
                output.push('}');
            }
        }
        rest = after;
    }
    output.push_str(rest);
    if missing && has_recorded_modifiers {
        return t_for(
            language,
            "This effect's evaluated modifiers were recorded with the hit. See the snapshot values below.",
        );
    }
    if missing {
        output.push('\n');
        output.push_str(&t_for(
            language,
            "The snapshot did not record these dynamic values.",
        ));
    }
    if filled {
        output.push('\n');
        output.push_str(&t_for(
            language,
            "Description values are from resource data, not observed magnitudes.",
        ));
    }
    output
}

impl Catalog {
    fn localize(&self, effect: &mut Effect, language: Language) {
        let entry = self.entries.get(&key(&effect.key));
        let recorded = effect
            .modifiers
            .as_ref()
            .and_then(|capture| capture.values())
            .is_some_and(|values| !values.is_empty());
        let native_description = strip_tags(&effect.description);
        let native_name = strip_tags(&effect.name);
        let missing_name = native_name.is_empty() || key(&native_name) == key(&effect.key);
        if let Some(entry) = entry {
            let name = strip_tags(&entry.name);
            let origin_label = match entry.origin {
                Origin::Ability => Some("Skill effect: {0}"),
                Origin::Awakening => Some("Awakening effect: {0}"),
                Origin::Arc => Some("Arc effect: {0}"),
                Origin::Equipment => Some("Set effect: {0}"),
                Origin::Buff | Origin::Tag => None,
            };
            if missing_name {
                effect.name = match origin_label {
                    Some(label) => tf_for(language, label, &[&name]),
                    None if entry.origin == Origin::Tag => t_for(language, &name),
                    None => name,
                };
            } else {
                effect.name = native_name;
            }
            let resource_description = strip_tags(&entry.description);
            let template = if native_description.is_empty() {
                &resource_description
            } else {
                &native_description
            };
            // Never apply current resource parameters to a different historical template.
            let parameters = if !recorded && template == &resource_description {
                entry.parameters.as_slice()
            } else {
                &[]
            };
            effect.description = format_description(template, parameters, recorded, language);
            if native_description.is_empty()
                && origin_label.is_some()
                && !effect.description.is_empty()
            {
                effect.description = format!(
                    "{}\n{}",
                    t_for(language, "Related resource description:"),
                    effect.description
                );
            }
        } else {
            effect.name = if native_name.is_empty() {
                effect.key.clone()
            } else {
                native_name
            };
            effect.description = format_description(&native_description, &[], recorded, language);
        }
        if effect.description.is_empty() {
            effect.description = t_for(
                language,
                if recorded {
                    "This effect's evaluated modifiers were recorded with the hit. See the snapshot values below."
                } else {
                    "No verified effect description is available."
                },
            );
        }
    }
}

pub fn localize_snapshot(
    snapshot: &mut PluginHitSnapshot,
    language: Language,
) -> Result<(), EffectTextError> {
    if let Some(data) = &mut snapshot.data {
        let catalog = CATALOG.as_ref().map_err(|e| *e)?;
        for effects in [&mut data.attacker_effects, &mut data.victim_effects]
            .into_iter()
            .flatten()
        {
            for effect in effects.effects.iter_mut().flatten() {
                catalog.localize(effect, language);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn effect(key: &str) -> Effect {
        Effect {
            instance_key: "frozen".into(),
            key: key.into(),
            name: key.into(),
            description: String::new(),
            source: "source".into(),
            kind: 255,
            stacks: 1,
            duration: -1.0,
            start_world_time: 12.0,
            level: 1.0,
            inhibited: false,
            modifiers: None,
        }
    }
    #[test]
    fn bundled_references_name_effects_without_guessing_from_identifiers() {
        let catalog = CATALOG.as_ref().unwrap();
        for (key, name) in [
            ("Buff_Oneiroi075_Level5_C", "策略特性：清晰"),
            ("Buff_Oneiroi075_Level1_C", "重复囚徒困境"),
            ("Buff_Nanally010_Passive1_C", "不止「一腔」的热血"),
            ("Buff_Oneiroi075_Passive1_C", "镜象"),
            ("Buff_Oneiroi075_Passive2_C", "交感性神经系统"),
            ("Buff_Sagiri003_Passive2_C", "可以吃吗？"),
            ("Buff_Fork_xiangdao_5_1_C", "灵感终结者"),
            ("Buff_Equipment_Nature2_4_C", "森林萤火之心"),
            ("GE_CoolDownBase_C", "技能冷却状态"),
        ] {
            let mut row = effect(key);
            catalog.localize(&mut row, Language::SimplifiedChinese);
            assert!(row.name.contains(name), "{}", row.name);
            assert_eq!(row.key, key);
            assert_eq!(row.kind, 255);
            assert!(!row.description.contains("{0}"));
        }
    }
    #[test]
    fn resource_parameters_and_unrecorded_runtime_magnitudes_stay_distinct() {
        let catalog = CATALOG.as_ref().unwrap();
        let mut cooldown = effect("GE_Sagiri_BranchCD1_C");
        catalog.localize(&mut cooldown, Language::SimplifiedChinese);
        assert!(cooldown.description.contains("180"));
        assert!(cooldown.description.contains("非当时实测数值"));
        let mut dynamic = effect("Buff_Fork_mofeikesi_1_3_C");
        catalog.localize(&mut dynamic, Language::SimplifiedChinese);
        assert!(dynamic.description.contains("数值未记录"));
        assert!(!dynamic.description.contains("{0}"));
        let mut static_bonus = effect("Buff_Cang023_Level1_SkillDamUp_C");
        catalog.localize(&mut static_bonus, Language::SimplifiedChinese);
        assert!(static_bonus.description.contains("30%"));
        for key in catalog.entries.keys() {
            let mut row = effect(key);
            catalog.localize(&mut row, Language::SimplifiedChinese);
            assert!(!row.description.contains("{0}"), "{key}");
        }
        let mut captured = effect("Buff_Fork_mofeikesi_1_3_C");
        captured.modifiers = Some(
            crate::engine::model::plugin_snapshot::EffectModifiers::Recorded {
                values: vec![crate::engine::model::plugin_snapshot::EffectModifier {
                    attribute: "AtkUp".into(),
                    operation:
                        crate::engine::model::plugin_snapshot::EffectModifierOperation::AddBase,
                    magnitude: 0.125,
                }],
            },
        );
        catalog.localize(&mut captured, Language::SimplifiedChinese);
        assert!(captured.description.contains("快照数值"));
        assert!(!captured.description.contains("数值未记录"));
        assert_eq!(
            captured.modifiers.as_ref().unwrap().values().unwrap()[0].magnitude,
            0.125
        );
        let mut observed = effect("Buff_Fork_mofeikesi_1_3_C");
        observed.name = "历史名称".into();
        observed.description = "攻击力提升12%，持续20秒。".into();
        catalog.localize(&mut observed, Language::SimplifiedChinese);
        assert_eq!(observed.name, "历史名称");
        assert_eq!(observed.description, "攻击力提升12%，持续20秒。");
    }
}
