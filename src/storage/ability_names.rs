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
    ABILITY_TIPS_PATH, AbilityCatalog, SKILL_DAMAGE_DATA_PATH, find_data_file,
    load_ability_tip_names,
};
use crate::storage::i18n::Language;

#[derive(Default)]
struct Store {
    catalog: Arc<AbilityCatalog>,
    /// ability name -> localized display text; reloaded on language switch.
    ability_tip_names: HashMap<String, String>,
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
        Ok(catalog) => Arc::new(catalog),
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
    let mut store = STORE.write().expect("ability name store lock poisoned");
    store.catalog = Arc::clone(&catalog);
    store.ability_tip_names = ability_tip_names;
    let warning = (!warnings.is_empty()).then(|| warnings.join("; "));
    (catalog, warning)
}

/// Reloads just the localized names for the new `language`; the catalog is
/// structural and doesn't need reloading.
pub fn reload(language: Language) -> Option<String> {
    let (ability_tip_names, warning) = match load_map(ABILITY_TIPS_PATH, |path| {
        load_ability_tip_names(path, language)
    }) {
        Ok(names) => (names, None),
        Err(error) => (HashMap::new(), Some(error.to_string())),
    };
    STORE
        .write()
        .expect("ability name store lock poisoned")
        .ability_tip_names = ability_tip_names;
    warning
}

pub fn resolve_damage_name(effect_name: &str) -> Option<String> {
    let store = STORE.read().expect("ability name store lock poisoned");
    resolve_from_maps(&store.catalog, &store.ability_tip_names, effect_name)
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
) {
    let mut store = STORE.write().expect("ability name store lock poisoned");
    store.catalog = Arc::new(AbilityCatalog::from(skills));
    store.ability_tip_names = ability_tip_names;
}

fn resolve_from_maps(
    catalog: &AbilityCatalog,
    ability_tip_names: &HashMap<String, String>,
    effect_name: &str,
) -> Option<String> {
    ability_tip_names
        .get(catalog.ability_name(effect_name)?)
        .cloned()
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
                "GE_Player_Sagiri_UltraSkill1_Damage"
            ),
            Some("Feast of Gluttony".to_owned())
        );
        assert_eq!(
            resolve_from_maps(&catalog, &ability_tip_names, "GE_Unknown_Effect"),
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
            resolve_from_maps(&catalog, &english, effect_name).as_deref(),
            Some("Feast of Gluttony")
        );
        assert_eq!(
            resolve_from_maps(&catalog, &chinese, effect_name).as_deref(),
            Some("盛宴之刻")
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
