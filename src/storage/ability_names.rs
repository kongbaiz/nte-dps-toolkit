//! Live re-resolution of skill/ability display names.
//!
//! [`crate::engine::capture::PacketDecoder`] bakes a hit's `damage_name` into the
//! `Hit` at capture time using whatever UI language was active then, so switching
//! the language afterward can't retroactively fix already-captured hits without a
//! full re-import. This store mirrors the same two resources
//! (`skill_damage.json`, `ability_tips.json`) at the app layer instead, so the
//! display layer can re-resolve a name from a hit's language-independent
//! `gameplay_effect_name` against whatever language is active *right now* —
//! the same fix `crate::storage::i18n` already applies to UI chrome text.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, RwLock};

use crate::engine::parser::{
    ABILITY_TIPS_PATH, GameplayEffectSkill, SKILL_DAMAGE_DATA_PATH, find_data_file,
    load_ability_tip_names, load_gameplay_effect_skills,
};
use crate::storage::i18n::Language;

#[derive(Default)]
struct Store {
    /// effect name -> ability name; structural, not language-dependent.
    skills: HashMap<String, GameplayEffectSkill>,
    /// ability name -> localized display text; reloaded on language switch.
    ability_tip_names: HashMap<String, String>,
}

static STORE: LazyLock<RwLock<Store>> = LazyLock::new(|| RwLock::new(Store::default()));

fn load_map<T: Default>(relative_path: &str, loader: impl FnOnce(&Path) -> anyhow::Result<T>) -> T {
    find_data_file(Path::new(relative_path))
        .and_then(|path| loader(&path).ok())
        .unwrap_or_default()
}

/// Loads both resources for the given `language`. Call once at startup.
pub fn init(language: Language) {
    let skills = load_map(SKILL_DAMAGE_DATA_PATH, load_gameplay_effect_skills);
    let ability_tip_names = load_map(ABILITY_TIPS_PATH, |path| {
        load_ability_tip_names(path, language)
    });
    let mut store = STORE.write().expect("ability name store lock poisoned");
    store.skills = skills;
    store.ability_tip_names = ability_tip_names;
}

/// Reloads just the localized names for the new `language`; `skills` is
/// structural and doesn't need reloading.
pub fn reload(language: Language) {
    let ability_tip_names = load_map(ABILITY_TIPS_PATH, |path| {
        load_ability_tip_names(path, language)
    });
    STORE
        .write()
        .expect("ability name store lock poisoned")
        .ability_tip_names = ability_tip_names;
}

/// Re-resolves a hit's display name from its `gameplay_effect_name` using the
/// *current* language, independent of whatever was baked into `damage_name` at
/// capture time. Returns `None` when the effect isn't in the skill table (e.g.
/// reaction-only effects, which route through a different label already).
pub fn resolve_damage_name(effect_name: &str) -> Option<String> {
    let store = STORE.read().expect("ability name store lock poisoned");
    resolve_from_maps(&store.skills, &store.ability_tip_names, effect_name)
}

/// Seeds the global store directly, bypassing file I/O, for tests elsewhere
/// (e.g. [`crate::app::hud`]) that need to exercise live resolution end-to-end.
#[cfg(test)]
pub(crate) fn set_for_test(
    skills: HashMap<String, GameplayEffectSkill>,
    ability_tip_names: HashMap<String, String>,
) {
    let mut store = STORE.write().expect("ability name store lock poisoned");
    store.skills = skills;
    store.ability_tip_names = ability_tip_names;
}

fn resolve_from_maps(
    skills: &HashMap<String, GameplayEffectSkill>,
    ability_tip_names: &HashMap<String, String>,
    effect_name: &str,
) -> Option<String> {
    let ability_name = skills.get(effect_name)?.ability_name.as_deref()?;
    ability_tip_names.get(ability_name).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_through_effect_then_ability_name() {
        let skills = HashMap::from([(
            "GE_Player_Sagiri_UltraSkill1_Damage".to_owned(),
            GameplayEffectSkill {
                damage_source_category: Some("Q".to_owned()),
                ability_name: Some("GA_Sagiri_UltraSkill".to_owned()),
                attack_type: "Q技能".to_owned(),
            },
        )]);
        let ability_tip_names = HashMap::from([(
            "GA_Sagiri_UltraSkill".to_owned(),
            "Feast of Gluttony".to_owned(),
        )]);

        assert_eq!(
            resolve_from_maps(
                &skills,
                &ability_tip_names,
                "GE_Player_Sagiri_UltraSkill1_Damage"
            ),
            Some("Feast of Gluttony".to_owned())
        );
        assert_eq!(
            resolve_from_maps(&skills, &ability_tip_names, "GE_Unknown_Effect"),
            None
        );
    }
}
