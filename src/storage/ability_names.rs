//! Live resolution of localized skill/ability display names.
//!
//! The parser stores stable ability and GameplayEffect identifiers. This store
//! combines the structural ability catalog with the active language's
//! `ability_tips.json` entries so only the display layer produces localized
//! names.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::engine::parser::{
    ABILITY_TIPS_PATH, AbilityCatalog, GAMEPLAY_EFFECT_SEMANTICS_PATH, SKILL_DAMAGE_DATA_PATH,
    find_data_file, load_ability_tip_names, load_gameplay_effect_semantic_names,
};
use crate::storage::i18n::Language;

#[derive(Default)]
struct Store {
    catalog: Arc<AbilityCatalog>,
    /// False only for an empty poison-recovery fallback that requires `init`
    /// before a localized-only reload can be considered complete.
    catalog_initialized: bool,
    /// ability name -> localized display text; reloaded on language switch.
    ability_tip_names: HashMap<String, String>,
    /// GameplayEffect name -> (localized component text, show parent ability).
    semantic_names: HashMap<String, (String, bool)>,
}

static STORE: LazyLock<RwLock<Store>> = LazyLock::new(|| RwLock::new(Store::default()));

/// Ability display data is a rebuildable projection. A poisoned write may
/// contain a mixed catalog/name generation, so discard it wholesale rather
/// than exposing the poisoned guard's contents.
fn write_rebuildable_store(store: &RwLock<Store>) -> RwLockWriteGuard<'_, Store> {
    match store.write() {
        Ok(guard) => guard,
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            *guard = Store::default();
            store.clear_poison();
            guard
        }
    }
}

fn read_rebuildable_store(store: &RwLock<Store>) -> RwLockReadGuard<'_, Store> {
    loop {
        match store.read() {
            Ok(guard) => return guard,
            Err(poisoned) => {
                drop(poisoned);
                drop(write_rebuildable_store(store));
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReloadDisposition {
    Applied,
    RequiresFullInit,
}

fn replace_initialized_store(
    store: &RwLock<Store>,
    catalog: Arc<AbilityCatalog>,
    ability_tip_names: HashMap<String, String>,
    semantic_names: HashMap<String, (String, bool)>,
) {
    let mut store = write_rebuildable_store(store);
    store.catalog = catalog;
    store.catalog_initialized = true;
    store.ability_tip_names = ability_tip_names;
    store.semantic_names = semantic_names;
}

fn install_reloaded_names(
    store: &RwLock<Store>,
    ability_tip_names: HashMap<String, String>,
    semantic_names: HashMap<String, (String, bool)>,
) -> ReloadDisposition {
    let mut store = write_rebuildable_store(store);
    if !store.catalog_initialized {
        return ReloadDisposition::RequiresFullInit;
    }
    store.ability_tip_names = ability_tip_names;
    store.semantic_names = semantic_names;
    ReloadDisposition::Applied
}

fn load_map<T>(
    relative_path: &str,
    loader: impl FnOnce(&Path) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let path = find_data_file(Path::new(relative_path))
        .ok_or_else(|| anyhow::anyhow!("missing resource {relative_path}"))?;
    loader(&path)
}

fn repair_stale_zankou_names(names: &mut HashMap<String, String>, language: Language) {
    // Current CN v1.2.1 names from the game's Zankou skill table. Some bundled
    // ability_tips rows still contain Mitsuki's older music-themed labels.
    // For non-CN languages, keep any future non-stale translation; until Global
    // catches up, prefer the correct CN name over a confidently wrong old label.
    let fixes: [(&str, &str, &[&str]); 7] = [
        ("GA_Zankou_Melee", "燎原", &["清唱", "Cappella", "アカペラ"]),
        ("GA_Zankou_MeleeMagic", "燎原", &[""]),
        (
            "GA_Zankou_Skill",
            "绯影闪",
            &["华彩唱段", "Cadenza Aria", "華麗なるカデンツァ"],
        ),
        (
            "GA_Zankou_UltraSkill",
            "焚天烬灭舞",
            &["众声轮唱", "Canon Chorus", "カノン"],
        ),
        (
            "GA_Zankou_QTE",
            "饲火",
            &["不协和音", "Dissonance", "不協和音"],
        ),
        (
            "GA_Zankou_Passive1",
            "暮落残阳",
            &["泛音", "Harmonics", "ハーモニクス"],
        ),
        (
            "GA_Zankou_Passive2",
            "殷红幻景",
            &["泛音", "Harmonics", "ハーモニクス"],
        ),
    ];

    for (ability_id, cn_name, stale_names) in fixes {
        let current = names
            .get(ability_id)
            .map(String::as_str)
            .unwrap_or_default();
        if language == Language::SimplifiedChinese
            || current.is_empty()
            || stale_names.contains(&current)
        {
            names.insert(ability_id.to_owned(), cn_name.to_owned());
        }
    }
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
    let mut ability_tip_names = match load_map(ABILITY_TIPS_PATH, |path| {
        load_ability_tip_names(path, language)
    }) {
        Ok(names) => names,
        Err(error) => {
            warnings.push(error.to_string());
            HashMap::new()
        }
    };
    repair_stale_zankou_names(&mut ability_tip_names, language);
    let semantic_names = match load_map(GAMEPLAY_EFFECT_SEMANTICS_PATH, |path| {
        load_gameplay_effect_semantic_names(path, language)
    }) {
        Ok(names) => names,
        Err(error) => {
            warnings.push(error.to_string());
            HashMap::new()
        }
    };
    replace_initialized_store(
        &STORE,
        Arc::clone(&catalog),
        ability_tip_names,
        semantic_names,
    );
    let warning = (!warnings.is_empty()).then(|| warnings.join("; "));
    (catalog, warning)
}

/// Reloads just the localized names for the new `language`; the catalog is
/// structural and doesn't need reloading.
pub fn reload(language: Language) -> Option<String> {
    let mut warnings = Vec::new();
    let mut ability_tip_names = match load_map(ABILITY_TIPS_PATH, |path| {
        load_ability_tip_names(path, language)
    }) {
        Ok(names) => names,
        Err(error) => {
            warnings.push(error.to_string());
            HashMap::new()
        }
    };
    repair_stale_zankou_names(&mut ability_tip_names, language);
    let semantic_names = match load_map(GAMEPLAY_EFFECT_SEMANTICS_PATH, |path| {
        load_gameplay_effect_semantic_names(path, language)
    }) {
        Ok(names) => names,
        Err(error) => {
            warnings.push(error.to_string());
            HashMap::new()
        }
    };
    if install_reloaded_names(&STORE, ability_tip_names, semantic_names)
        == ReloadDisposition::RequiresFullInit
    {
        return init(language).1;
    }
    (!warnings.is_empty()).then(|| warnings.join("; "))
}

pub fn resolve_damage_name(effect_name: &str) -> Option<String> {
    let store = read_rebuildable_store(&STORE);
    resolve_from_maps(
        &store.catalog,
        &store.ability_tip_names,
        &store.semantic_names,
        effect_name,
    )
}

pub fn resolve_ability_name(ability_name: &str) -> Option<String> {
    read_rebuildable_store(&STORE)
        .ability_tip_names
        .get(ability_name)
        .cloned()
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
    fn repairs_stale_zankou_names_without_overwriting_future_translations() {
        let mut chinese = HashMap::from([
            ("GA_Zankou_Melee".to_owned(), "清唱".to_owned()),
            ("GA_Zankou_Skill".to_owned(), "华彩唱段".to_owned()),
            ("GA_Zankou_UltraSkill".to_owned(), "众声轮唱".to_owned()),
            ("GA_Zankou_QTE".to_owned(), "不协和音".to_owned()),
            ("GA_Zankou_Passive1".to_owned(), "泛音".to_owned()),
            ("GA_Zankou_Passive2".to_owned(), "泛音".to_owned()),
        ]);
        repair_stale_zankou_names(&mut chinese, Language::SimplifiedChinese);
        assert_eq!(chinese["GA_Zankou_Melee"], "燎原");
        assert_eq!(chinese["GA_Zankou_Skill"], "绯影闪");
        assert_eq!(chinese["GA_Zankou_UltraSkill"], "焚天烬灭舞");
        assert_eq!(chinese["GA_Zankou_QTE"], "饲火");
        assert_eq!(chinese["GA_Zankou_Passive1"], "暮落残阳");
        assert_eq!(chinese["GA_Zankou_Passive2"], "殷红幻景");

        let mut english = HashMap::from([
            ("GA_Zankou_Melee".to_owned(), "Cappella".to_owned()),
            (
                "GA_Zankou_Skill".to_owned(),
                "Future Correct Translation".to_owned(),
            ),
        ]);
        repair_stale_zankou_names(&mut english, Language::English);
        assert_eq!(english["GA_Zankou_Melee"], "燎原");
        assert_eq!(english["GA_Zankou_Skill"], "Future Correct Translation");
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

    #[test]
    fn poisoned_ability_store_discards_partial_catalog_and_names() {
        let store = RwLock::new(Store {
            catalog: Arc::new(catalog()),
            catalog_initialized: true,
            ability_tip_names: HashMap::from([("ability".to_owned(), "partial".to_owned())]),
            semantic_names: HashMap::from([("effect".to_owned(), ("partial".to_owned(), true))]),
        });
        let _ = std::panic::catch_unwind(|| {
            let mut guard = store.write().expect("test ability store");
            guard
                .ability_tip_names
                .insert("leaked".to_owned(), "value".to_owned());
            panic!("poison test ability store");
        });

        let guard = read_rebuildable_store(&store);
        assert!(
            guard
                .catalog
                .ability_name("GE_Player_Sagiri_UltraSkill1_Damage")
                .is_none()
        );
        assert!(guard.ability_tip_names.is_empty());
        assert!(guard.semantic_names.is_empty());
        assert!(!guard.catalog_initialized);
        drop(guard);
        assert!(!store.is_poisoned());

        assert_eq!(
            install_reloaded_names(
                &store,
                HashMap::from([("GA_Sagiri_UltraSkill".to_owned(), "restored".to_owned())]),
                HashMap::new(),
            ),
            ReloadDisposition::RequiresFullInit
        );
        replace_initialized_store(
            &store,
            Arc::new(catalog()),
            HashMap::from([("GA_Sagiri_UltraSkill".to_owned(), "restored".to_owned())]),
            HashMap::new(),
        );
        let guard = read_rebuildable_store(&store);
        assert!(guard.catalog_initialized);
        assert_eq!(
            resolve_from_maps(
                &guard.catalog,
                &guard.ability_tip_names,
                &guard.semantic_names,
                "GE_Player_Sagiri_UltraSkill1_Damage",
            )
            .as_deref(),
            Some("restored")
        );
    }
}
