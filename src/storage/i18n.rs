//! Lightweight UI localization.
//!
//! The source keeps every user-facing string in **English**; that English text is
//! the lookup key. A localized build overlays a `res/languages/<code>.json` map of
//! `"English key" -> "localized value"`. English therefore needs no file: when the
//! active language is [`Language::English`] (or a key is missing from a locale map)
//! the key itself is returned unchanged.
//!
//! Game-specific proper nouns and descriptions were sourced from the official
//! `NTE_Assets` localization when a match existed, and left at their original value
//! otherwise; both simply live as entries in the locale JSON.
//!
//! The store is a process-wide [`RwLock`] because desktop commands and background
//! workers may both format status text. Swapping the
//! language (settings dropdown / startup) reloads the map in place.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

use serde::{Deserialize, Serialize};

use crate::storage::resource::{BoundedResourceTextError, read_resource_text_bounded};

const MAX_LOCALE_RESOURCE_BYTES: usize = 1024 * 1024;
const MAX_LOCALE_ENTRIES: usize = 4096;
const MAX_LOCALE_FIELD_BYTES: usize = 1024;

/// Languages the UI can render. English is the key language; every other variant
/// has a matching `res/languages/<code>.json`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[serde(rename = "en")]
    English,
    #[serde(rename = "ja")]
    Japanese,
    /// Default so existing (Chinese-only) installs keep their current UI; English
    /// is opt-in via the settings dropdown.
    #[default]
    #[serde(rename = "zh-CN")]
    SimplifiedChinese,
}

const LANGUAGES: [Language; 3] = [
    Language::English,
    Language::Japanese,
    Language::SimplifiedChinese,
];

impl Language {
    pub fn all() -> &'static [Self] {
        &LANGUAGES
    }

    /// Stable code used for the config value and the `res/languages/<code>.json` filename.
    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Japanese => "ja",
            Self::SimplifiedChinese => "zh-CN",
        }
    }

    /// Endonym shown in the language dropdown, written in the language itself so a
    /// user can find their language without already reading the current one.
    pub fn native_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Japanese => "日本語",
            Self::SimplifiedChinese => "简体中文",
        }
    }

    /// Folder name used by localized reaction-text images under
    /// `res/images/font/tiaozi1/<folder>/`.
    pub fn reaction_text_folder(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Japanese => "ja",
            Self::SimplifiedChinese => "zh",
        }
    }

    /// Resource path of the overlay map, or `None` for the key language (English).
    fn resource_path(self) -> Option<String> {
        match self {
            Self::English => None,
            other => Some(format!("res/languages/{}.json", other.code())),
        }
    }

    /// Match a Windows locale name (e.g. `"zh-CN"`, `"ja-JP"`) to a supported UI
    /// language by primary subtag, falling back to English when nothing matches.
    fn from_locale_name(locale: &str) -> Self {
        let primary = locale
            .split(['-', '_'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        Self::all()
            .iter()
            .copied()
            .find(|lang| lang.code().split(['-', '_']).next() == Some(primary.as_str()))
            .unwrap_or(Self::English)
    }

    /// Best-effort default UI language for a brand-new install (no `config.json`
    /// yet): the system locale if a matching localization file exists, else
    /// English. Only consulted the first time the config is created — later
    /// launches always use the persisted `language` value, so this never
    /// overrides a user's own choice or an existing (pre-i18n) install's
    /// historical Simplified Chinese default.
    pub fn system_default() -> Self {
        crate::platform::locale::system_locale_name()
            .map(|locale| Self::from_locale_name(&locale))
            .unwrap_or(Self::English)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocaleLoadDiagnostic {
    ReadFailed,
    TooLarge,
    InvalidUtf8,
    InvalidJson,
    InvalidShape,
    StateRecovered,
}

impl LocaleLoadDiagnostic {
    pub const fn code(self) -> &'static str {
        match self {
            Self::ReadFailed => "resource_read_failed",
            Self::TooLarge => "resource_too_large",
            Self::InvalidUtf8 => "resource_invalid_utf8",
            Self::InvalidJson => "resource_invalid_json",
            Self::InvalidShape => "resource_invalid_shape",
            Self::StateRecovered => "state_recovered",
        }
    }
}

#[derive(Default)]
struct Store {
    language: Language,
    /// `"English key" -> "localized value"`; empty for English.
    map: HashMap<String, String>,
    diagnostic: Option<LocaleLoadDiagnostic>,
    auxiliary_diagnostic: Option<LocaleLoadDiagnostic>,
}

impl Store {
    fn poison_fallback() -> Self {
        Self {
            language: Language::English,
            map: HashMap::new(),
            diagnostic: Some(LocaleLoadDiagnostic::StateRecovered),
            auxiliary_diagnostic: None,
        }
    }
}

#[derive(Default)]
struct LocaleOverlay {
    map: HashMap<String, String>,
    diagnostic: Option<LocaleLoadDiagnostic>,
}

static STORE: LazyLock<RwLock<Store>> = LazyLock::new(|| RwLock::new(Store::default()));
static SIMPLIFIED_CHINESE_OVERLAY: LazyLock<LocaleOverlay> =
    LazyLock::new(|| load_overlay(Language::SimplifiedChinese));
static JAPANESE_OVERLAY: LazyLock<LocaleOverlay> =
    LazyLock::new(|| load_overlay(Language::Japanese));

/// Locale data is a rebuildable projection. If a writer panics, discard the
/// entire partially-updated projection before allowing any later read/write.
fn write_rebuildable_store(store: &RwLock<Store>) -> RwLockWriteGuard<'_, Store> {
    match store.write() {
        Ok(guard) => guard,
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            *guard = Store::poison_fallback();
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

/// Load the overlay map for `language`. Missing/invalid files degrade to an empty
/// map (keys fall back to their English text) and retain one bounded diagnostic.
fn load_overlay(language: Language) -> LocaleOverlay {
    let Some(path) = language.resource_path() else {
        return LocaleOverlay::default();
    };
    let text = match read_resource_text_bounded(Path::new(&path), MAX_LOCALE_RESOURCE_BYTES) {
        Ok(text) => text,
        Err(error) => {
            let diagnostic = match error {
                BoundedResourceTextError::ReadFailed => LocaleLoadDiagnostic::ReadFailed,
                BoundedResourceTextError::TooLarge => LocaleLoadDiagnostic::TooLarge,
                BoundedResourceTextError::InvalidUtf8 => LocaleLoadDiagnostic::InvalidUtf8,
            };
            return LocaleOverlay {
                diagnostic: Some(diagnostic),
                ..LocaleOverlay::default()
            };
        }
    };
    match parse_overlay_text(&text) {
        Ok(map) => LocaleOverlay {
            map,
            diagnostic: None,
        },
        Err(diagnostic) => LocaleOverlay {
            diagnostic: Some(diagnostic),
            ..LocaleOverlay::default()
        },
    }
}

fn parse_overlay_text(text: &str) -> Result<HashMap<String, String>, LocaleLoadDiagnostic> {
    let map = serde_json::from_str::<HashMap<String, String>>(text)
        .map_err(|_| LocaleLoadDiagnostic::InvalidJson)?;
    if map.len() > MAX_LOCALE_ENTRIES
        || map.iter().any(|(key, value)| {
            key.is_empty()
                || key.len() > MAX_LOCALE_FIELD_BYTES
                || value.len() > MAX_LOCALE_FIELD_BYTES
        })
    {
        return Err(LocaleLoadDiagnostic::InvalidShape);
    }
    Ok(map)
}

/// Switch the active UI language and load its overlay map. Call once at startup and
/// whenever the settings dropdown changes.
pub fn set_language(language: Language) {
    let overlay = load_overlay(language);
    let mut store = write_rebuildable_store(&STORE);
    store.language = language;
    store.map = overlay.map;
    store.diagnostic = overlay.diagnostic;
}

/// The active UI language. Lets non-UI display helpers pick a localized field
/// without threading the setting through every call.
pub fn current_language() -> Language {
    read_rebuildable_store(&STORE).language
}

/// The current bounded localization degradation marker, without a path, JSON
/// payload, or parser detail crossing the diagnostics boundary.
pub fn locale_load_diagnostic() -> Option<LocaleLoadDiagnostic> {
    locale_load_diagnostic_for(&STORE)
}

fn locale_load_diagnostic_for(store: &RwLock<Store>) -> Option<LocaleLoadDiagnostic> {
    let store = read_rebuildable_store(store);
    store.diagnostic.or(store.auxiliary_diagnostic)
}

fn record_auxiliary_locale_diagnostic(diagnostic: Option<LocaleLoadDiagnostic>) {
    record_auxiliary_locale_diagnostic_for(&STORE, diagnostic);
}

fn record_auxiliary_locale_diagnostic_for(
    store: &RwLock<Store>,
    diagnostic: Option<LocaleLoadDiagnostic>,
) {
    let Some(diagnostic) = diagnostic else {
        return;
    };
    write_rebuildable_store(store).auxiliary_diagnostic = Some(diagnostic);
}

/// Translate an English key into the active language. Returns the key unchanged for
/// English or when the locale map has no entry for it.
pub fn t(key: &str) -> String {
    let store = read_rebuildable_store(&STORE);
    if matches!(store.language, Language::English) {
        return key.to_owned();
    }
    match store.map.get(key) {
        Some(value) => value.clone(),
        None => key.to_owned(),
    }
}

/// Translate without changing the active UI language. Command search uses this
/// to match both the English source key and Simplified Chinese in every locale.
pub fn t_for(language: Language, key: &str) -> String {
    let overlay = match language {
        Language::English => return key.to_owned(),
        Language::Japanese => &*JAPANESE_OVERLAY,
        Language::SimplifiedChinese => &*SIMPLIFIED_CHINESE_OVERLAY,
    };
    record_auxiliary_locale_diagnostic(overlay.diagnostic);
    overlay
        .map
        .get(key)
        .cloned()
        .unwrap_or_else(|| key.to_owned())
}

/// Translate `key`, then apply the shared Rust/TypeScript placeholder grammar.
///
/// `{n}` reuses argument `n` anywhere in the template. When argument `n` has no
/// indexed token, it consumes the next `{}` token. Unknown or unfilled tokens are
/// left literal. Runtime substitution is required because the template comes from
/// the locale map rather than a compile-time `format!` string.
pub fn tf(key: &str, args: &[&str]) -> String {
    let template = t(key);
    format_template(&template, args)
}

fn format_template(template: &str, args: &[&str]) -> String {
    let mut message = template.to_owned();
    for (index, argument) in args.iter().enumerate() {
        let indexed = format!("{{{index}}}");
        if message.contains(&indexed) {
            message = message.replace(&indexed, argument);
        } else if let Some(position) = message.find("{}") {
            message.replace_range(position..position + 2, argument);
        }
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct PlaceholderConformanceCase {
        template: String,
        arguments: Vec<String>,
        expected: String,
    }

    #[test]
    fn placeholder_formatter_matches_the_shared_conformance_corpus() {
        let cases: Vec<PlaceholderConformanceCase> = serde_json::from_str(include_str!(
            "../../res/languages/placeholder-conformance.json"
        ))
        .expect("placeholder conformance corpus should be valid JSON");

        for case in cases {
            let arguments = case
                .arguments
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            assert_eq!(
                format_template(&case.template, &arguments),
                case.expected,
                "template: {}",
                case.template
            );
        }
    }

    fn placeholder_arity(template: &str) -> usize {
        let bytes = template.as_bytes();
        let mut indexed = std::collections::BTreeSet::new();
        let mut sequential = 0usize;
        let mut cursor = 0usize;
        while cursor < bytes.len() {
            if bytes[cursor] != b'{' {
                cursor += 1;
                continue;
            }
            let Some(relative_end) = bytes[cursor + 1..].iter().position(|byte| *byte == b'}')
            else {
                break;
            };
            let end = cursor + 1 + relative_end;
            let token = &bytes[cursor + 1..end];
            if token.is_empty() {
                sequential += 1;
            } else if token.iter().all(u8::is_ascii_digit) {
                let token = std::str::from_utf8(token).expect("ASCII placeholder index");
                indexed.insert(
                    token
                        .parse::<usize>()
                        .expect("placeholder index should fit usize"),
                );
            }
            cursor = end + 1;
        }

        let mut arity = indexed.iter().next_back().map_or(0, |index| index + 1);
        while (0..arity).filter(|index| !indexed.contains(index)).count() < sequential {
            arity += 1;
        }
        arity
    }

    #[test]
    fn every_locale_preserves_the_source_placeholder_arity() {
        for language in [Language::SimplifiedChinese, Language::Japanese] {
            for (key, translation) in load_overlay(language).map {
                assert_eq!(
                    placeholder_arity(&translation),
                    placeholder_arity(&key),
                    "{} placeholder mismatch for key {key:?}: {translation:?}",
                    language.code()
                );
            }
        }
    }

    #[test]
    fn language_codes_and_names_are_stable() {
        assert_eq!(Language::English.code(), "en");
        assert_eq!(Language::Japanese.code(), "ja");
        assert_eq!(Language::SimplifiedChinese.code(), "zh-CN");
        assert_eq!(Language::English.native_name(), "English");
        assert_eq!(Language::Japanese.native_name(), "日本語");
        assert_eq!(Language::SimplifiedChinese.native_name(), "简体中文");
        assert_eq!(Language::SimplifiedChinese.reaction_text_folder(), "zh");
        assert_eq!(Language::default(), Language::SimplifiedChinese);
    }

    #[test]
    fn english_returns_key_unchanged() {
        assert_eq!(t_for(Language::English, "Settings"), "Settings");
        assert_eq!(format_template("Loaded {} rows", &["12"]), "Loaded 12 rows");
    }

    #[test]
    fn config_serializes_to_stable_codes() {
        assert_eq!(
            serde_json::to_string(&Language::SimplifiedChinese).unwrap(),
            "\"zh-CN\""
        );
        assert_eq!(
            serde_json::from_str::<Language>("\"en\"").unwrap(),
            Language::English
        );
        assert_eq!(
            serde_json::from_str::<Language>("\"ja\"").unwrap(),
            Language::Japanese
        );
    }

    #[test]
    fn tf_leaves_extra_placeholders_literal() {
        assert_eq!(format_template("{} of {}", &["3"]), "3 of {}");
    }

    #[test]
    fn translating_for_search_does_not_change_the_active_language() {
        let before = current_language();
        assert_eq!(t_for(Language::SimplifiedChinese, "Settings"), "设置");
        assert_eq!(current_language(), before);
    }

    #[test]
    fn system_default_matches_locale_by_primary_subtag() {
        assert_eq!(
            Language::from_locale_name("zh-CN"),
            Language::SimplifiedChinese
        );
        // Traditional-Chinese locales fall back to the only Chinese file shipped.
        assert_eq!(
            Language::from_locale_name("zh-TW"),
            Language::SimplifiedChinese
        );
        assert_eq!(Language::from_locale_name("ja-JP"), Language::Japanese);
        assert_eq!(Language::from_locale_name("en-US"), Language::English);
        assert_eq!(Language::from_locale_name("fr-FR"), Language::English);
        assert_eq!(Language::from_locale_name(""), Language::English);
    }

    #[test]
    fn japanese_locale_covers_every_simplified_chinese_key() {
        // zh-CN.json is the most complete locale map today; ja.json should
        // have a translation for every key it defines so switching to
        // Japanese doesn't silently fall back to raw English key text.
        let zh_map = load_overlay(Language::SimplifiedChinese).map;
        let ja_map = load_overlay(Language::Japanese).map;
        assert!(!zh_map.is_empty());
        assert!(!ja_map.is_empty());

        let missing: Vec<&String> = zh_map
            .keys()
            .filter(|key| !ja_map.contains_key(*key))
            .collect();
        assert!(missing.is_empty(), "ja.json is missing keys: {missing:?}");
    }

    #[test]
    fn malformed_or_structurally_unbounded_locale_maps_return_stable_diagnostics() {
        assert_eq!(
            parse_overlay_text("not json"),
            Err(LocaleLoadDiagnostic::InvalidJson)
        );
        let oversized_key = "x".repeat(MAX_LOCALE_FIELD_BYTES + 1);
        let oversized = serde_json::to_string(&HashMap::from([(oversized_key, "ok")]))
            .expect("locale test JSON");
        assert_eq!(
            parse_overlay_text(&oversized),
            Err(LocaleLoadDiagnostic::InvalidShape)
        );
        assert_eq!(LocaleLoadDiagnostic::TooLarge.code(), "resource_too_large");
    }

    #[test]
    fn auxiliary_locale_failure_is_visible_without_a_healthy_false_positive() {
        let store = RwLock::new(Store::default());

        record_auxiliary_locale_diagnostic_for(&store, None);
        assert_eq!(locale_load_diagnostic_for(&store), None);

        record_auxiliary_locale_diagnostic_for(&store, Some(LocaleLoadDiagnostic::InvalidJson));
        assert_eq!(
            locale_load_diagnostic_for(&store),
            Some(LocaleLoadDiagnostic::InvalidJson)
        );

        record_auxiliary_locale_diagnostic_for(&store, None);
        assert_eq!(
            locale_load_diagnostic_for(&store),
            Some(LocaleLoadDiagnostic::InvalidJson),
            "a later healthy auxiliary lookup must not erase an observed failure"
        );
    }

    #[test]
    fn poisoned_locale_store_discards_partial_state_before_reads_continue() {
        let store = RwLock::new(Store {
            language: Language::Japanese,
            map: HashMap::from([("Settings".to_owned(), "partial".to_owned())]),
            diagnostic: None,
            auxiliary_diagnostic: None,
        });
        let _ = std::panic::catch_unwind(|| {
            let mut guard = store.write().expect("test locale store");
            guard.language = Language::Japanese;
            guard.map.insert("leaked".to_owned(), "value".to_owned());
            panic!("poison test locale store");
        });

        let guard = read_rebuildable_store(&store);
        assert_eq!(guard.language, Language::English);
        assert!(guard.map.is_empty());
        assert_eq!(guard.diagnostic, Some(LocaleLoadDiagnostic::StateRecovered));
        drop(guard);
        assert!(!store.is_poisoned());
    }
}
