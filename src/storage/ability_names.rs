//! Live resolution of localized skill/ability display names.
//!
//! The parser stores stable ability and GameplayEffect identifiers. This store
//! combines the structural ability catalog with the active language's
//! `ability_tips.json` entries so only the display layer produces localized
//! names.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock, RwLock};

use crate::engine::parser::{
    ABILITY_TIPS_PATH, AbilityCatalog, GAMEPLAY_EFFECT_SEMANTICS_PATH, SKILL_DAMAGE_DATA_PATH,
    find_data_file, load_ability_tip_names, load_gameplay_effect_semantic_names,
};
use crate::storage::i18n::Language;

#[derive(Default)]
struct Store {
    catalog: Arc<AbilityCatalog>,
    /// ability name -> localized display text; reloaded on language switch.
    ability_tip_names: HashMap<String, String>,
    /// GameplayEffect name -> (localized component text, show parent ability).
    semantic_names: HashMap<String, (String, bool)>,
}

static STORE: LazyLock<RwLock<Store>> = LazyLock::new(|| RwLock::new(Store::default()));

fn load_map<T>(
    relative_path: &str,
    loader: impl FnOnce(&Path) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let path = find_data_file(Path::new(relative_path))
        .ok_or_else(|| anyhow::anyhow!("missing resource {relative_path}"))?;
    loader(&path)
}

/// Loads both resources for the given `language`. Call once at startup and pass
/// the returned structural catalog to capture/replay decoders.
pub fn init(language: Language) -> (Arc<AbilityCatalog>, Option<String>) {
    let mut warnings = Vec::new();
    let catalog = match load_map(SKILL_DAMAGE_DATA_PATH, AbilityCatalog::load) {
        Ok(mut catalog) => {
            match load_map(GAMEPLAY_EFFECT_SEMANTICS_PATH, |path| {
                catalog.apply_semantics(path)
            }) {
                Ok(()) => {}
                Err(error) => warnings.push(error.to_string()),
            }
            Arc::new(catalog)
        }
        Err(error) => {
            warnings.push(error.to_string());
            Arc::new(AbilityCatalog::default())
        }
    };
    let ability_tip_names = match load_map(ABILITY_TIPS_PATH, |path| {
        load_ability_tip_names(path, language)
    }) {
        Ok(names) => names,
        Err(error) => {
            warnings.push(error.to_string());
            HashMap::new()
        }
    };
    let semantic_names = match load_map(GAMEPLAY_EFFECT_SEMANTICS_PATH, |path| {
        load_gameplay_effect_semantic_names(path, language)
    }) {
        Ok(names) => names,
        Err(error) => {
            warnings.push(error.to_string());
            HashMap::new()
        }
    };
    let mut store = STORE.write().expect("ability name store lock poisoned");
    store.catalog = Arc::clone(&catalog);
    store.ability_tip_names = ability_tip_names;
    store.semantic_names = semantic_names;
    let warning = (!warnings.is_empty()).then(|| warnings.join("; "));
    (catalog, warning)
}

/// Reloads just the localized names for the new `language`; the catalog is
/// structural and doesn't need reloading.
pub fn reload(language: Language) -> Option<String> {
    let mut warnings = Vec::new();
    let ability_tip_names = match load_map(ABILITY_TIPS_PATH, |path| {
        load_ability_tip_names(path, language)
    }) {
        Ok(names) => names,
        Err(error) => {
            warnings.push(error.to_string());
            HashMap::new()
        }
    };
    let semantic_names = match load_map(GAMEPLAY_EFFECT_SEMANTICS_PATH, |path| {
        load_gameplay_effect_semantic_names(path, language)
    }) {
        Ok(names) => names,
        Err(error) => {
            warnings.push(error.to_string());
            HashMap::new()
        }
    };
    let mut store = STORE.write().expect("ability name store lock poisoned");
    store.ability_tip_names = ability_tip_names;
    store.semantic_names = semantic_names;
    (!warnings.is_empty()).then(|| warnings.join("; "))
}

pub fn resolve_damage_name(effect_name: &str) -> Option<String> {
    let store = STORE.read().expect("ability name store lock poisoned");
    resolve_from_maps(
        &store.catalog,
        &store.ability_tip_names,
        &store.semantic_names,
        effect_name,
    )
}

pub fn resolve_ability_name(ability_name: &str) -> Option<String> {
    STORE
        .read()
        .expect("ability name store lock poisoned")
        .ability_tip_names
        .get(ability_name)
        .cloned()
}

#[cfg(all(test, feature = "gui"))]
pub(crate) fn set_for_test(
    skills: HashMap<String, crate::engine::parser::GameplayEffectSkill>,
    ability_tip_names: HashMap<String, String>,
    semantic_names: HashMap<String, (String, bool)>,
) {
    let mut store = STORE.write().expect("ability name store lock poisoned");
    store.catalog = Arc::new(AbilityCatalog::from(skills));
    store.ability_tip_names = ability_tip_names;
    store.semantic_names = semantic_names;
}

fn resolve_from_maps(
    catalog: &AbilityCatalog,
    ability_tip_names: &HashMap<String, String>,
    semantic_names: &HashMap<String, (String, bool)>,
    effect_name: &str,
) -> Option<String> {
    let ability_name = catalog
        .ability_name(effect_name)
        .and_then(|ability_name| ability_tip_names.get(ability_name))
        .cloned();
    let Some((component_name, show_parent_ability)) = semantic_names.get(effect_name) else {
        return ability_name;
    };
    if *show_parent_ability && let Some(ability_name) = ability_name {
        return Some(format!("{ability_name} · {component_name}"));
    }
    Some(component_name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::parser::GameplayEffectSkill;

    fn catalog() -> AbilityCatalog {
        AbilityCatalog::from(HashMap::from([(
            "GE_Player_Sagiri_UltraSkill1_Damage".to_owned(),
            GameplayEffectSkill {
                damage_source_category: Some("Q".to_owned()),
                ability_name: Some("GA_Sagiri_UltraSkill".to_owned()),
                attack_type: "Q技能".to_owned(),
                damage_component: None,
                owner_character_id: None,
            },
        )]))
    }

    #[test]
    fn resolves_through_effect_then_ability_name() {
        let catalog = catalog();
        let ability_tip_names = HashMap::from([(
            "GA_Sagiri_UltraSkill".to_owned(),
            "Feast of Gluttony".to_owned(),
        )]);

        assert_eq!(
            resolve_from_maps(
                &catalog,
                &ability_tip_names,
                &HashMap::new(),
                "GE_Player_Sagiri_UltraSkill1_Damage"
            ),
            Some("Feast of Gluttony".to_owned())
        );
        assert_eq!(
            resolve_from_maps(
                &catalog,
                &ability_tip_names,
                &HashMap::new(),
                "GE_Unknown_Effect"
            ),
            None
        );
    }

    #[test]
    fn localized_names_share_the_same_structural_ability_identity() {
        let catalog = catalog();
        let effect_name = "GE_Player_Sagiri_UltraSkill1_Damage";
        let ability_name = "GA_Sagiri_UltraSkill";
        let english = HashMap::from([(ability_name.to_owned(), "Feast of Gluttony".to_owned())]);
        let chinese = HashMap::from([(ability_name.to_owned(), "盛宴之刻".to_owned())]);

        assert_eq!(catalog.ability_name(effect_name), Some(ability_name));
        assert_eq!(
            resolve_from_maps(&catalog, &english, &HashMap::new(), effect_name).as_deref(),
            Some("Feast of Gluttony")
        );
        assert_eq!(
            resolve_from_maps(&catalog, &chinese, &HashMap::new(), effect_name).as_deref(),
            Some("盛宴之刻")
        );
    }

    #[test]
    fn semantic_component_joins_parent_ability_name() {
        let catalog = catalog();
        let effect_name = "GE_Player_Sagiri_UltraSkill1_Damage";
        let abilities = HashMap::from([("GA_Sagiri_UltraSkill".to_owned(), "盛宴之刻".to_owned())]);
        let components = HashMap::from([(effect_name.to_owned(), ("追加攻击".to_owned(), true))]);

        assert_eq!(
            resolve_from_maps(&catalog, &abilities, &components, effect_name).as_deref(),
            Some("盛宴之刻 · 追加攻击")
        );
    }

    #[test]
    fn semantic_component_can_hide_parent_ability_name() {
        let catalog = catalog();
        let effect_name = "GE_Player_Sagiri_UltraSkill1_Damage";
        let abilities = HashMap::from([("GA_Sagiri_UltraSkill".to_owned(), "盛宴之刻".to_owned())]);
        let components =
            HashMap::from([(effect_name.to_owned(), ("「噩梦」伤害".to_owned(), false))]);

        assert_eq!(
            resolve_from_maps(&catalog, &abilities, &components, effect_name).as_deref(),
            Some("「噩梦」伤害")
        );
    }

    #[test]
    fn load_map_preserves_missing_resource_error() {
        let error = load_map::<AbilityCatalog>(
            "res/data/skills/missing_ability_catalog.json",
            AbilityCatalog::load,
        )
        .expect_err("missing structural skill data must remain visible");

        assert!(error.to_string().contains("missing resource"));
    }
}
