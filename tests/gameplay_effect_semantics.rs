use std::fs;
use std::path::Path;

use serde_json::Value;

fn load_json(relative: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("invalid JSON in {}: {error}", path.display()))
}

#[test]
fn semantic_components_do_not_repeat_the_parent_ability_name() {
    let semantics = load_json("res/data/skills/gameplay_effect_semantics.json");
    let ability_tips = load_json("res/data/skills/ability_tips.json");

    let effects = semantics["effects"]
        .as_object()
        .expect("semantics effects must be an object");
    let abilities = ability_tips["abilities"]
        .as_object()
        .expect("ability tips must contain an abilities object");

    for (effect_name, semantic) in effects {
        if semantic.get("show_parent_ability").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        let Some(ability_id) = semantic.get("ability").and_then(Value::as_str) else {
            continue;
        };
        let Some(parent_name) = abilities
            .get(ability_id)
            .and_then(|ability| ability.get("name_zh"))
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        let Some(component_name) = semantic.get("damage_name_zh").and_then(Value::as_str) else {
            continue;
        };

        for separator in ["·", "・", ":", "：", " "] {
            let duplicated_prefix = format!("{parent_name}{separator}");
            assert!(
                !component_name.starts_with(&duplicated_prefix),
                "{effect_name} embeds parent ability {parent_name:?} in component {component_name:?}"
            );
        }
    }
}

#[test]
fn zankou_dot_semantics_follow_current_cn_assets() {
    let semantics = load_json("res/data/skills/gameplay_effect_semantics.json");
    let skill_damage = load_json("res/data/skills/skill_damage.json");
    let effects = &semantics["effects"];

    let dot = &effects["GE_Player_Zankou_DotDamage"];
    let dot_skill = &skill_damage[0]["Rows"]["GE_Player_Zankou_DotDamage"];
    assert_eq!(dot_skill["GAName"], "GA_Zankou_Melee");
    assert_eq!(dot["owner_character_id"], 1036);
    assert!(dot.get("ability").is_none());
    assert!(dot.get("attack_type").is_none());
    assert_eq!(dot["show_parent_ability"], false);
    assert_eq!(dot["damage_name_zh"], "蚀心");

    let ultra_dot = &effects["GE_Player_Zankou_DotUltraDamage"];
    let ultra_dot_skill = &skill_damage[0]["Rows"]["GE_Player_Zankou_DotUltraDamage"];
    assert_eq!(ultra_dot_skill["GAName"], "GA_Zankou_UltraSkill");
    assert_eq!(ultra_dot["owner_character_id"], 1036);
    assert!(ultra_dot.get("ability").is_none());
    assert!(ultra_dot.get("attack_type").is_none());
    assert_eq!(ultra_dot["show_parent_ability"], false);
    assert_eq!(ultra_dot["damage_name_zh"], "鸩火");
}
