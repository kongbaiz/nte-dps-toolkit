use std::borrow::Cow;
#[cfg(feature = "desktop")]
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[cfg(feature = "desktop")]
use crate::engine::model::CharacterInfo;
use anyhow::{Context, Result, anyhow};

include!(concat!(env!("OUT_DIR"), "/embedded_resources.rs"));

#[cfg(feature = "desktop")]
const MODS_PLUGIN_PATH: &str = "plugins/dwmapi.dll";
#[cfg(feature = "desktop")]
const MODS_PLUGIN_REQUIRED_MOD_RUNTIME_SYMBOLS: [&[u8]; 21] = [
    b"NTE_DPS_TOOL_MODS_PLUGIN_V1",
    b"game.session",
    b"game.player_controller",
    b"game.player_state",
    b"combat_clock.pause_mask",
    b"combat_clock.state_flags",
    b"memory.read_f32_milli",
    b"memory.read_fname_hash",
    b"cache.get",
    b"cache.remember",
    b"memory.write_u64",
    b"unreal.reflection",
    b"unreal.find_function",
    b"unreal.params_clear",
    b"unreal.params_write_u64",
    b"unreal.params_read_u64",
    b"unreal.call",
    b"process.event",
    b"unreal.watch",
    b"unreal.unwatch",
    b"event.next",
];

pub(crate) fn bundled_resource(path: &str) -> Option<&'static [u8]> {
    embedded_resource(path)
}

pub(crate) fn resource_file_path(path: &Path) -> Option<PathBuf> {
    disk_resource_candidates(path)
        .into_iter()
        .find(|candidate| candidate.is_file())
}

#[cfg(feature = "desktop")]
pub fn read_mods_plugin() -> std::io::Result<Option<Vec<u8>>> {
    let relative_path = Path::new(MODS_PLUGIN_PATH);
    let candidates = [
        super::paths::software_dir().join(relative_path),
        Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path),
    ];
    read_first_compatible_mods_plugin(&candidates)
}

#[cfg(feature = "desktop")]
fn read_first_compatible_mods_plugin(candidates: &[PathBuf]) -> std::io::Result<Option<Vec<u8>>> {
    let mut incompatible = None;
    for candidate in candidates {
        match std::fs::read(candidate) {
            Ok(bytes) if mods_plugin_supports_bundled_mods(&bytes) => return Ok(Some(bytes)),
            Ok(_) => {
                incompatible.get_or_insert(candidate);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    if let Some(path) = incompatible {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "{} does not provide the Mod runtime APIs required by the bundled scripts",
                path.display()
            ),
        ));
    }
    Ok(None)
}

#[cfg(feature = "desktop")]
fn mods_plugin_supports_bundled_mods(plugin: &[u8]) -> bool {
    MODS_PLUGIN_REQUIRED_MOD_RUNTIME_SYMBOLS
        .iter()
        .all(|symbol| plugin.windows(symbol.len()).any(|window| window == *symbol))
}

pub(crate) fn resource_exists(path: &Path) -> bool {
    resource_file_path(path).is_some() || bundled_resource_for_path(path).is_some()
}

pub(crate) fn read_resource_text(path: &Path) -> Result<String> {
    let bytes = read_resource_bytes(path)?;
    String::from_utf8(bytes.into_owned())
        .with_context(|| format!("资源不是 UTF-8 文本 {}", path.display()))
}

pub(crate) fn read_resource_bytes(path: &Path) -> Result<Cow<'static, [u8]>> {
    if let Some(disk_path) = resource_file_path(path) {
        let bytes = std::fs::read(&disk_path)
            .with_context(|| format!("无法读取资源 {}", disk_path.display()))?;
        return Ok(Cow::Owned(bytes));
    }

    if let Some(bytes) = bundled_resource_for_path(path) {
        return Ok(Cow::Borrowed(bytes));
    }

    Err(anyhow!("找不到资源 {}", path.display()))
}

#[cfg(feature = "desktop")]
const DISTINCT_CHARACTER_PALETTE: [[u8; 3]; 32] = [
    [0xE8, 0x17, 0x4B],
    [0x17, 0xE8, 0x17],
    [0xE0, 0x5C, 0xD5],
    [0x17, 0xC5, 0xE8],
    [0xE8, 0x4B, 0x17],
    [0x17, 0x17, 0xE8],
    [0x1F, 0xA3, 0x61],
    [0x11, 0x54, 0xB0],
    [0xA3, 0x56, 0x1F],
    [0xB4, 0xE0, 0x5C],
    [0xA3, 0x1F, 0x61],
    [0x61, 0x1F, 0xA3],
    [0x4F, 0xEE, 0xD3],
    [0xE8, 0xB4, 0x17],
    [0xC5, 0x17, 0xE8],
    [0x4F, 0xEE, 0x76],
    [0x11, 0x7B, 0xB0],
    [0xE8, 0xE8, 0x17],
    [0x40, 0xA3, 0x1F],
    [0xE0, 0xA9, 0x5C],
    [0x17, 0x4B, 0xE8],
    [0x1F, 0xA3, 0x98],
    [0xE8, 0x17, 0x91],
    [0xE0, 0x67, 0x5C],
    [0xA3, 0xA3, 0x1F],
    [0xE8, 0x80, 0x17],
    [0x91, 0xE8, 0x17],
    [0x17, 0xE8, 0xA2],
    [0x4F, 0x91, 0xEE],
    [0xA3, 0x1F, 0x35],
    [0xA3, 0x1F, 0x8D],
    [0x9E, 0x4F, 0xEE],
];

/// Assign every character without an explicit color a unique catalog-wide
/// accent. Character identity and catalog order are the only inputs; combat
/// rank, avatar pixels, and attributes cannot make teammates converge.
#[cfg(feature = "desktop")]
pub(crate) fn assign_missing_character_colors(characters: &mut HashMap<u32, CharacterInfo>) {
    let mut character_ids = characters.keys().copied().collect::<Vec<_>>();
    character_ids.sort_unstable();
    let mut used = characters
        .values()
        .filter_map(|character| character.color.as_deref())
        .filter_map(parse_character_hex_rgb)
        .collect::<HashSet<_>>();

    for (catalog_index, character_id) in character_ids.into_iter().enumerate() {
        let Some(character) = characters.get_mut(&character_id) else {
            continue;
        };
        if character
            .color
            .as_deref()
            .and_then(parse_character_hex_rgb)
            .is_some()
        {
            continue;
        }
        let color = available_character_rgb(catalog_index, character_id, &used);
        used.insert(color);
        character.color = Some(format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2]));
    }
}

#[cfg(feature = "desktop")]
fn parse_character_hex_rgb(value: &str) -> Option<[u8; 3]> {
    let value = value.strip_prefix('#')?;
    if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some([
        u8::from_str_radix(&value[0..2], 16).ok()?,
        u8::from_str_radix(&value[2..4], 16).ok()?,
        u8::from_str_radix(&value[4..6], 16).ok()?,
    ])
}

#[cfg(feature = "desktop")]
fn available_character_rgb(
    catalog_index: usize,
    character_id: u32,
    used: &HashSet<[u8; 3]>,
) -> [u8; 3] {
    for offset in 0..DISTINCT_CHARACTER_PALETTE.len() {
        let color =
            DISTINCT_CHARACTER_PALETTE[(catalog_index + offset) % DISTINCT_CHARACTER_PALETTE.len()];
        if !used.contains(&color) {
            return color;
        }
    }

    let mut salt = 0_u32;
    loop {
        let hash = character_id
            .wrapping_mul(0x9E37_79B9)
            .wrapping_add(salt.wrapping_mul(0x85EB_CA6B));
        let color = [
            48 + (hash & 0xAF) as u8,
            48 + ((hash >> 8) & 0xAF) as u8,
            48 + ((hash >> 16) & 0xAF) as u8,
        ];
        if !used.contains(&color) {
            return color;
        }
        salt = salt.wrapping_add(1);
    }
}

fn bundled_resource_for_path(path: &Path) -> Option<&'static [u8]> {
    let key = embedded_resource_key(path)?;
    bundled_resource(&key)
}

fn disk_resource_candidates(path: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    push_unique_path(&mut candidates, path.to_path_buf());

    if !path.is_absolute() {
        if let Ok(current_dir) = std::env::current_dir() {
            push_unique_path(&mut candidates, current_dir.join(path));
        }

        if let Ok(executable) = std::env::current_exe() {
            for ancestor in executable.ancestors().skip(1) {
                push_unique_path(&mut candidates, ancestor.join(path));
            }
        }

        push_unique_path(
            &mut candidates,
            Path::new(env!("CARGO_MANIFEST_DIR")).join(path),
        );
    }

    candidates
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

fn embedded_resource_key(path: &Path) -> Option<String> {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let normalized = normalized.trim_start_matches("./");
    if normalized == "res" || normalized.starts_with("res/") {
        return Some(normalized.to_owned());
    }
    normalized
        .find("/res/")
        .map(|index| normalized[index + 1..].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    #[cfg(feature = "desktop")]
    fn character_colors_are_unique_and_independent_of_hashmap_order() {
        let character = |id| CharacterInfo {
            name_zh: format!("角色{id}"),
            name_en: format!("Character {id}"),
            color: None,
            avatar: None,
            attribute: Some("同属性".to_owned()),
        };
        let ids = (1000..1022).collect::<Vec<_>>();
        let mut forward = ids
            .iter()
            .copied()
            .map(|id| (id, character(id)))
            .collect::<HashMap<_, _>>();
        let mut reverse = ids
            .iter()
            .rev()
            .copied()
            .map(|id| (id, character(id)))
            .collect::<HashMap<_, _>>();

        assign_missing_character_colors(&mut forward);
        assign_missing_character_colors(&mut reverse);

        let colors = ids
            .iter()
            .map(|id| forward[id].color.as_deref().unwrap())
            .collect::<HashSet<_>>();
        assert_eq!(colors.len(), ids.len());
        for id in ids {
            assert_eq!(forward[&id].color, reverse[&id].color);
        }
    }

    #[test]
    #[cfg(not(feature = "external_resources"))]
    fn bundled_resource_contains_character_data() {
        let bytes = bundled_resource("res/data/characters/characters.json")
            .expect("characters.json should be bundled");

        assert!(std::str::from_utf8(bytes).unwrap().contains("characters"));
    }

    #[test]
    #[cfg(not(feature = "external_resources"))]
    fn missing_res_path_falls_back_to_bundled_resource() {
        let path = Path::new("missing-root/res/data/characters/characters.json");
        let text = read_resource_text(path).expect("bundled characters should load");

        assert!(text.contains("characters"));
    }

    #[test]
    fn disk_file_wins_over_matching_bundled_resource_key() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("nte-resource-test-{}-{unique}", std::process::id()));
        let path = root.join("res/data/characters/characters.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "disk wins").unwrap();

        let text = read_resource_text(&path).expect("disk resource should load");

        assert_eq!(text, "disk wins");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(feature = "desktop")]
    fn mods_plugin_loader_reads_the_first_compatible_candidate() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "nte-plugin-source-test-{}-{unique}",
            std::process::id()
        ));
        let missing = root.join("missing/dwmapi.dll");
        let plugin = root.join("plugins/dwmapi.dll");
        std::fs::create_dir_all(plugin.parent().unwrap()).unwrap();
        std::fs::write(&plugin, compatible_mods_plugin()).unwrap();

        let bytes = read_first_compatible_mods_plugin(&[missing, plugin]).unwrap();

        assert_eq!(bytes.as_deref(), Some(compatible_mods_plugin().as_slice()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(feature = "desktop")]
    fn mods_plugin_loader_skips_an_incompatible_runtime_package() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "nte-plugin-compatibility-test-{}-{unique}",
            std::process::id()
        ));
        let stale = root.join("stale/dwmapi.dll");
        let current = root.join("current/dwmapi.dll");
        std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
        std::fs::create_dir_all(current.parent().unwrap()).unwrap();
        std::fs::write(&stale, b"old plugin").unwrap();
        std::fs::write(&current, compatible_mods_plugin()).unwrap();

        let bytes = read_first_compatible_mods_plugin(&[stale, current]).unwrap();

        assert_eq!(bytes.as_deref(), Some(compatible_mods_plugin().as_slice()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(feature = "desktop")]
    fn mods_plugin_loader_reports_an_incompatible_runtime_package() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "nte-plugin-incompatible-test-{}-{unique}",
            std::process::id()
        ));
        let stale = root.join("plugins/dwmapi.dll");
        std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
        std::fs::write(&stale, b"old plugin").unwrap();

        let error = read_first_compatible_mods_plugin(&[stale])
            .expect_err("an incompatible plugin should be rejected");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("Mod runtime APIs"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "desktop")]
    fn compatible_mods_plugin() -> Vec<u8> {
        MODS_PLUGIN_REQUIRED_MOD_RUNTIME_SYMBOLS
            .iter()
            .flat_map(|symbol| symbol.iter().copied().chain([0]))
            .collect()
    }

    #[test]
    #[cfg(all(feature = "cli", not(feature = "external_resources")))]
    fn cli_bundle_contains_only_core_data_resources() {
        for path in [
            "res/data/characters/characters.json",
            "res/data/equipment/equipment.json",
            "res/data/skills/ability_tips.json",
            "res/data/skills/gameplay_effect_mapping.json",
            "res/data/skills/skill_damage.json",
        ] {
            assert!(
                bundled_resource(path).is_some(),
                "missing core resource {path}"
            );
        }
        assert!(bundled_resource("res/data/abyss/abyss_monsters.json").is_none());
        assert!(bundled_resource("res/images/characters/player_003_256.png").is_none());
        assert!(bundled_resource("res/icons/app-icon.png").is_none());
    }

    #[test]
    #[cfg(all(feature = "desktop", not(feature = "external_resources")))]
    fn desktop_bundle_keeps_full_visual_resources() {
        assert!(bundled_resource("res/images/characters/player_003_256.png").is_some());
        assert!(bundled_resource("res/icons/app-icon.png").is_some());
    }
}
