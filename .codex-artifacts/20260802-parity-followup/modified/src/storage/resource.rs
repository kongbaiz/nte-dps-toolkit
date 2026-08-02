use std::borrow::Cow;
#[cfg(any(feature = "desktop", feature = "gui"))]
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[cfg(any(feature = "desktop", feature = "gui"))]
use crate::engine::model::CharacterInfo;
use anyhow::{Context, Result, anyhow};

include!(concat!(env!("OUT_DIR"), "/embedded_resources.rs"));

#[cfg(any(feature = "desktop", feature = "gui"))]
const MODS_PLUGIN_PATH: &str = "plugins/dwmapi.dll";
#[cfg(any(feature = "desktop", feature = "gui"))]
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

#[cfg(any(feature = "desktop", feature = "gui"))]
pub fn read_mods_plugin() -> std::io::Result<Option<Vec<u8>>> {
    let relative_path = Path::new(MODS_PLUGIN_PATH);
    let candidates = [
        super::paths::software_dir().join(relative_path),
        Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path),
    ];
    read_first_compatible_mods_plugin(&candidates)
}

#[cfg(any(feature = "desktop", feature = "gui"))]
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

#[cfg(any(feature = "desktop", feature = "gui"))]
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

/// Populate missing character colors with the same deterministic avatar-pixel
/// projection used by the established egui UI. Keeping this at the resource
/// boundary gives every desktop frontend one stable color for a character.
#[cfg(any(feature = "desktop", feature = "gui"))]
pub(crate) fn fill_missing_character_colors_from_avatars(
    characters: &mut HashMap<u32, CharacterInfo>,
    root: &Path,
) {
    let mut avatar_colors = HashMap::<String, [u8; 3]>::new();
    for character in characters.values_mut() {
        if character
            .color
            .as_deref()
            .is_some_and(valid_character_hex_color)
        {
            continue;
        }
        let Some(avatar) = character.avatar.as_deref() else {
            continue;
        };
        let color = *avatar_colors.entry(avatar.to_owned()).or_insert_with(|| {
            avatar_accent_rgb(root, avatar)
                .unwrap_or_else(|| deterministic_character_fallback_rgb(avatar.as_bytes(), false))
        });
        character.color = Some(format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2]));
    }
}

#[cfg(any(feature = "desktop", feature = "gui"))]
fn valid_character_hex_color(value: &str) -> bool {
    let value = value.strip_prefix('#').unwrap_or(value);
    value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(any(feature = "desktop", feature = "gui"))]
fn avatar_accent_rgb(root: &Path, resource_path: &str) -> Option<[u8; 3]> {
    let path = root.join(resource_path);
    let bytes = read_resource_bytes(&path)
        .or_else(|_| read_resource_bytes(Path::new(resource_path)))
        .ok()?;
    let image = image::load_from_memory(bytes.as_ref()).ok()?.to_rgba8();
    let mut red = 0.0_f64;
    let mut green = 0.0_f64;
    let mut blue = 0.0_f64;
    let mut total_weight = 0.0_f64;
    for pixel in image.pixels() {
        let [r, g, b, a] = pixel.0;
        if a < 128 {
            continue;
        }
        let rf = f64::from(r) / 255.0;
        let gf = f64::from(g) / 255.0;
        let bf = f64::from(b) / 255.0;
        let max = rf.max(gf).max(bf);
        let min = rf.min(gf).min(bf);
        let saturation = if max <= f64::EPSILON {
            0.0
        } else {
            (max - min) / max
        };
        if !(0.16..=0.96).contains(&max) || saturation < 0.16 {
            continue;
        }
        let mid_luma_weight = 1.0 - ((max - 0.58).abs() / 0.58).clamp(0.0, 0.85);
        let weight = saturation.powf(1.35) * mid_luma_weight.max(0.25) * f64::from(a) / 255.0;
        red += rf * weight;
        green += gf * weight;
        blue += bf * weight;
        total_weight += weight;
    }
    if total_weight <= f64::EPSILON {
        return None;
    }
    let mut r = red / total_weight;
    let mut g = green / total_weight;
    let mut b = blue / total_weight;
    let max = r.max(g).max(b).max(0.001);
    let min = r.min(g).min(b);
    let saturation = (max - min) / max;
    if saturation < 0.24 {
        let mean = (r + g + b) / 3.0;
        r = mean + (r - mean) * 1.45;
        g = mean + (g - mean) * 1.45;
        b = mean + (b - mean) * 1.45;
    }
    let max = r.max(g).max(b).max(0.001);
    if max < 0.46 {
        let scale = 0.46 / max;
        r *= scale;
        g *= scale;
        b *= scale;
    }
    Some([
        (r.clamp(0.0, 0.92) * 255.0).round() as u8,
        (g.clamp(0.0, 0.92) * 255.0).round() as u8,
        (b.clamp(0.0, 0.92) * 255.0).round() as u8,
    ])
}

#[cfg(any(feature = "desktop", feature = "gui"))]
pub(crate) fn deterministic_character_fallback_rgb(seed: &[u8], dark_mode: bool) -> [u8; 3] {
    let hash = seed.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    });
    let palette = if dark_mode {
        [
            [96, 165, 250],
            [167, 139, 250],
            [52, 211, 153],
            [251, 146, 60],
            [244, 114, 182],
            [34, 211, 238],
            [250, 204, 21],
            [248, 113, 113],
        ]
    } else {
        [
            [37, 99, 235],
            [124, 58, 237],
            [5, 150, 105],
            [194, 65, 12],
            [190, 24, 93],
            [8, 145, 178],
            [161, 98, 7],
            [190, 55, 65],
        ]
    };
    palette[hash as usize % palette.len()]
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
    #[cfg(any(feature = "desktop", feature = "gui"))]
    fn character_avatar_color_matches_the_established_projection() {
        let mut characters = HashMap::from([(
            1010,
            CharacterInfo {
                name_zh: "娜娜莉".to_owned(),
                name_en: "Nanally".to_owned(),
                color: None,
                avatar: Some("res/images/characters/player_010_256.png".to_owned()),
                attribute: None,
            },
        )]);

        fill_missing_character_colors_from_avatars(&mut characters, Path::new("."));

        assert_eq!(characters[&1010].color.as_deref(), Some("#A72648"));
    }

    #[test]
    #[cfg(any(feature = "desktop", feature = "gui"))]
    fn deterministic_character_fallback_is_stable() {
        assert_eq!(
            deterministic_character_fallback_rgb(&1010_u32.to_le_bytes(), false),
            deterministic_character_fallback_rgb(&1010_u32.to_le_bytes(), false)
        );
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
    #[cfg(feature = "gui")]
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
    #[cfg(feature = "gui")]
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
    #[cfg(feature = "gui")]
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

    #[cfg(feature = "gui")]
    fn compatible_mods_plugin() -> Vec<u8> {
        MODS_PLUGIN_REQUIRED_MOD_RUNTIME_SYMBOLS
            .iter()
            .flat_map(|symbol| symbol.iter().copied().chain([0]))
            .collect()
    }

    #[test]
    #[cfg(all(
        feature = "cli",
        not(feature = "gui"),
        not(feature = "external_resources")
    ))]
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
    #[cfg(all(feature = "gui", not(feature = "external_resources")))]
    fn gui_bundle_keeps_full_visual_resources() {
        assert!(bundled_resource("res/images/characters/player_003_256.png").is_some());
        assert!(bundled_resource("res/icons/app-icon.png").is_some());
    }
}
