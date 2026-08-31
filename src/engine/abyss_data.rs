use std::collections::HashMap;

use anyhow::{Context, Result};
use serde_json::Value;

const SUPPORTED_ABYSS_SEASONS: std::ops::RangeInclusive<u32> = 1..=10;
const MAX_REMOTE_JSON_NODES: usize = 500_000;
const MAX_REMOTE_JSON_DEPTH: usize = 48;
const MAX_REMOTE_STRING_BYTES: usize = 4 * 1024;
const MAX_REMOTE_TABLE_ROWS: usize = 50_000;
const MAX_REMOTE_SUMMARY_ROWS: usize = 10_000;
const MAX_REMOTE_SEASON_NAMES: usize = 32;
const MAX_REMOTE_DATASET_MONSTERS: usize = 20_000;

#[derive(Clone, Debug, Default)]
pub struct AbyssMonsterDataset {
    pub seasons: Vec<AbyssSeason>,
}

#[derive(Clone, Debug)]
pub struct AbyssSeason {
    pub season: u32,
    pub name: Option<String>,
    pub floors: Vec<AbyssFloor>,
}

#[derive(Clone, Debug)]
pub struct AbyssFloor {
    pub season: u32,
    pub season_name: Option<String>,
    pub floor: u32,
    pub name: Option<String>,
    pub monsters: Vec<AbyssMonsterEntry>,
    pub max_seconds: Option<f64>,
    pub star_thresholds: Vec<AbyssStarThreshold>,
    pub recommended_elements: AbyssRecommendedElements,
}

/// A clear-time cutoff sourced from `AbyssCloneLevelDataTable`'s
/// `PassConditions`: clearing the floor within `seconds` earns `stars`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AbyssStarThreshold {
    pub stars: u32,
    pub seconds: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AbyssRecommendedElements {
    pub first_half: Vec<String>,
    pub second_half: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct AbyssMonsterEntry {
    pub pack_id: String,
    pub attribute_id: String,
    pub monster_pool_id: Option<String>,
    pub monster_id: String,
    pub name: String,
    pub count: u32,
    pub level: Option<u32>,
    pub half: Option<u32>,
    pub wave: Option<u32>,
    pub is_boss: bool,
    pub stats: AbyssMonsterStats,
}

#[derive(Clone, Debug, Default)]
pub struct AbyssMonsterStats {
    pub hp_max_base: f64,
    pub raw_props: Vec<(String, f64)>,
}

#[derive(Clone, Debug, Default)]
struct StaticMonsterInfo {
    name: Option<String>,
}

impl AbyssMonsterDataset {
    /// Builds the runtime dataset from the four versioned tables delivered by
    /// the official abyss-data endpoint. The caller owns transport, archive,
    /// size and hash verification; this boundary enforces JSON shape/resource
    /// budgets before converting untrusted values into domain state.
    pub fn from_remote_tables(
        monster_static: &[u8],
        monster_pack: &[u8],
        floor_summary: &[u8],
        season_names: &[u8],
    ) -> Result<Self> {
        let static_rows = load_remote_rows(monster_static, "怪物静态表")?;
        let pack_rows = load_remote_rows(monster_pack, "怪物数值表")?;
        let summary_rows = load_remote_summary_rows(floor_summary)?;
        let season_names = load_remote_season_names(season_names)?;
        let static_index = build_static_index(&static_rows);
        let dataset =
            build_dataset_from_summary(&summary_rows, &pack_rows, &static_index, &season_names);
        validate_remote_dataset(&dataset)?;
        Ok(dataset)
    }

    pub fn season(&self, season: u32) -> Option<&AbyssSeason> {
        self.seasons.iter().find(|item| item.season == season)
    }

    pub fn floor(&self, season: u32, floor: u32) -> Option<&AbyssFloor> {
        self.season(season)
            .and_then(|season| season.floors.iter().find(|item| item.floor == floor))
    }

    pub fn monster(&self, pack_id: &str) -> Option<&AbyssMonsterEntry> {
        self.seasons
            .iter()
            .flat_map(|season| season.floors.iter())
            .flat_map(|floor| floor.monsters.iter())
            .find(|monster| monster.pack_id == pack_id)
    }
}

impl AbyssFloor {
    pub fn monster_count(&self) -> u32 {
        self.monsters.iter().map(|monster| monster.count).sum()
    }

    pub fn wave_count(&self) -> usize {
        self.monsters
            .iter()
            .filter_map(|monster| monster.wave)
            .collect::<std::collections::HashSet<_>>()
            .len()
    }
}

/// Per-floor metadata that is repeated across every wave/route row of a
/// summary floor (level name, clear-time budget, star thresholds, recommended
/// element types) and only needs to be captured once per (season, floor).
#[derive(Clone, Debug, Default)]
struct FloorMeta {
    name: Option<String>,
    max_seconds: Option<f64>,
    star_thresholds: Vec<AbyssStarThreshold>,
    recommended_elements: AbyssRecommendedElements,
}

fn build_dataset_from_summary(
    summary_rows: &[Value],
    pack_rows: &HashMap<String, Value>,
    static_index: &HashMap<String, StaticMonsterInfo>,
    season_names: &HashMap<u32, String>,
) -> AbyssMonsterDataset {
    let mut floors = HashMap::<(u32, u32), Vec<AbyssMonsterEntry>>::new();
    let mut floor_meta = HashMap::<(u32, u32), FloorMeta>::new();

    for row in summary_rows {
        let Some(abyss_key) = string(row, "abyss") else {
            continue;
        };
        let Some(season) = parse_abyss_group(abyss_key) else {
            continue;
        };
        let Some(floor) = u32_value(row, "level_id") else {
            continue;
        };
        floor_meta
            .entry((season, floor))
            .or_insert_with(|| floor_meta_from_row(row));
        let half = string(row, "route").and_then(parse_abyss_route_half);
        let wave = u32_value(row, "wave");
        let monster_pool_id = string(row, "monster_pool_id").map(str::to_owned);
        let Some(monsters) = row.get("monsters").and_then(Value::as_array) else {
            continue;
        };

        for (index, monster) in monsters.iter().enumerate() {
            let Some(attribute_id) = string(monster, "attribute_id") else {
                continue;
            };
            let monster_id = parse_abyss_attribute_monster_id(attribute_id)
                .or_else(|| {
                    monster_id_from_class_path(string(monster, "class").unwrap_or_default())
                })
                .unwrap_or_else(|| attribute_id.to_owned());
            let stats = pack_rows
                .get(attribute_id)
                .map(monster_stats)
                .unwrap_or_default();
            let static_info = lookup_static_monster(static_index, &monster_id);
            let name = string(monster, "name")
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned)
                .or_else(|| static_info.and_then(|info| info.name.clone()))
                .unwrap_or_else(|| monster_id.clone());
            let count = u32_value(monster, "count").unwrap_or(1).max(1);
            let pack_id = match &monster_pool_id {
                Some(pool) => format!(
                    "{pool}:{}:{}:{attribute_id}:{index}",
                    half.map_or_else(|| "-".to_owned(), |value| value.to_string()),
                    wave.map_or_else(|| "-".to_owned(), |value| value.to_string())
                ),
                None => attribute_id.to_owned(),
            };
            floors
                .entry((season, floor))
                .or_default()
                .push(AbyssMonsterEntry {
                    pack_id,
                    attribute_id: attribute_id.to_owned(),
                    monster_pool_id: monster_pool_id.clone(),
                    monster_id,
                    name,
                    count,
                    level: u32_value(monster, "level"),
                    half,
                    wave,
                    is_boss: bool_value(monster, "is_boss"),
                    stats,
                });
        }
    }

    build_dataset(floors, floor_meta, season_names)
}

fn floor_meta_from_row(row: &Value) -> FloorMeta {
    let name = string(row, "level_name")
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned);
    let max_seconds = f64_value(row, "max_seconds");
    let star_thresholds = row
        .get("star_thresholds")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    Some(AbyssStarThreshold {
                        stars: u32_value(entry, "stars")?,
                        seconds: f64_value(entry, "seconds")?,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let recommended_elements = row
        .get("recommended_elements")
        .map(|value| AbyssRecommendedElements {
            first_half: string_array(value, "first_half"),
            second_half: string_array(value, "second_half"),
        })
        .unwrap_or_default();
    FloorMeta {
        name,
        max_seconds,
        star_thresholds,
        recommended_elements,
    }
}

fn build_dataset(
    floors: HashMap<(u32, u32), Vec<AbyssMonsterEntry>>,
    floor_meta: HashMap<(u32, u32), FloorMeta>,
    season_names: &HashMap<u32, String>,
) -> AbyssMonsterDataset {
    let mut grouped = HashMap::<u32, Vec<AbyssFloor>>::new();
    for ((season, floor), mut monsters) in floors {
        if !is_supported_abyss_season(season) {
            continue;
        }
        monsters.sort_by(|left, right| {
            left.half
                .cmp(&right.half)
                .then_with(|| left.wave.cmp(&right.wave))
                .then_with(|| left.monster_pool_id.cmp(&right.monster_pool_id))
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.pack_id.cmp(&right.pack_id))
        });
        let season_name = season_names
            .get(&season)
            .cloned()
            .or_else(|| (season == 0).then(|| "通用配置".to_owned()));
        let meta = floor_meta
            .get(&(season, floor))
            .cloned()
            .unwrap_or_default();
        grouped.entry(season).or_default().push(AbyssFloor {
            season,
            season_name,
            floor,
            name: meta.name,
            monsters,
            max_seconds: meta.max_seconds,
            star_thresholds: meta.star_thresholds,
            recommended_elements: meta.recommended_elements,
        });
    }

    let mut seasons = grouped
        .into_iter()
        .map(|(season, mut floors)| {
            floors.sort_by_key(|floor| floor.floor);
            AbyssSeason {
                season,
                name: season_names
                    .get(&season)
                    .cloned()
                    .or_else(|| (season == 0).then(|| "通用配置".to_owned())),
                floors,
            }
        })
        .collect::<Vec<_>>();
    seasons.sort_by_key(|season| season.season);

    AbyssMonsterDataset { seasons }
}

fn is_supported_abyss_season(season: u32) -> bool {
    SUPPORTED_ABYSS_SEASONS.contains(&season)
}

fn load_remote_rows(bytes: &[u8], label: &str) -> Result<HashMap<String, Value>> {
    let document: Value =
        serde_json::from_slice(bytes).with_context(|| format!("{label}不是有效 JSON"))?;
    validate_remote_json_budget(&document)?;
    let rows = document
        .as_array()
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.get("Rows"))
        .and_then(Value::as_object)
        .with_context(|| format!("{label}缺少 Rows 对象"))?;
    anyhow::ensure!(rows.len() <= MAX_REMOTE_TABLE_ROWS, "{label}记录数超过上限");
    Ok(rows
        .iter()
        .map(|(key, row)| (key.clone(), row.clone()))
        .collect())
}

fn load_remote_summary_rows(bytes: &[u8]) -> Result<Vec<Value>> {
    let document: Value = serde_json::from_slice(bytes).context("深渊关卡汇总不是有效 JSON")?;
    validate_remote_json_budget(&document)?;
    let rows = document
        .get("rows")
        .and_then(Value::as_array)
        .context("深渊关卡汇总缺少 rows 数组")?;
    anyhow::ensure!(
        rows.len() <= MAX_REMOTE_SUMMARY_ROWS,
        "深渊关卡汇总记录数超过上限"
    );
    Ok(rows.clone())
}

fn load_remote_season_names(bytes: &[u8]) -> Result<HashMap<u32, String>> {
    let document: Value = serde_json::from_slice(bytes).context("深渊赛季名称不是有效 JSON")?;
    validate_remote_json_budget(&document)?;
    let names = parse_abyss_season_names(&document);
    anyhow::ensure!(
        names.len() <= MAX_REMOTE_SEASON_NAMES,
        "深渊赛季名称数量超过上限"
    );
    Ok(names)
}

fn validate_remote_json_budget(document: &Value) -> Result<()> {
    let mut stack = vec![(document, 0_usize)];
    let mut nodes = 0_usize;
    while let Some((value, depth)) = stack.pop() {
        nodes = nodes.saturating_add(1);
        anyhow::ensure!(nodes <= MAX_REMOTE_JSON_NODES, "深渊 JSON 节点数超过上限");
        anyhow::ensure!(depth <= MAX_REMOTE_JSON_DEPTH, "深渊 JSON 嵌套层级超过上限");
        match value {
            Value::String(text) => anyhow::ensure!(
                text.len() <= MAX_REMOTE_STRING_BYTES,
                "深渊 JSON 字符串长度超过上限"
            ),
            Value::Array(values) => {
                stack.extend(values.iter().map(|value| (value, depth + 1)));
            }
            Value::Object(values) => {
                for (key, value) in values {
                    anyhow::ensure!(
                        key.len() <= MAX_REMOTE_STRING_BYTES,
                        "深渊 JSON 字段名长度超过上限"
                    );
                    stack.push((value, depth + 1));
                }
            }
            Value::Number(number) => anyhow::ensure!(
                number.as_f64().is_some_and(f64::is_finite),
                "深渊 JSON 包含非法数值"
            ),
            Value::Null | Value::Bool(_) => {}
        }
    }
    Ok(())
}

fn validate_remote_dataset(dataset: &AbyssMonsterDataset) -> Result<()> {
    anyhow::ensure!(!dataset.seasons.is_empty(), "远程深渊数据没有受支持赛季");
    let mut monster_count = 0_usize;
    for season in &dataset.seasons {
        anyhow::ensure!(
            is_supported_abyss_season(season.season),
            "远程深渊数据包含不受支持赛季"
        );
        anyhow::ensure!(!season.floors.is_empty(), "远程深渊赛季没有关卡");
        for floor in &season.floors {
            anyhow::ensure!(!floor.monsters.is_empty(), "远程深渊关卡没有怪物");
            for monster in &floor.monsters {
                monster_count = monster_count.saturating_add(1);
                anyhow::ensure!(
                    monster_count <= MAX_REMOTE_DATASET_MONSTERS,
                    "远程深渊怪物记录数超过上限"
                );
                anyhow::ensure!(monster.count > 0, "远程深渊怪物数量非法");
                anyhow::ensure!(
                    monster.stats.hp_max_base.is_finite() && monster.stats.hp_max_base > 0.0,
                    "远程深渊怪物生命值非法"
                );
                anyhow::ensure!(
                    monster
                        .stats
                        .raw_props
                        .iter()
                        .all(|(_, value)| value.is_finite()),
                    "远程深渊怪物属性包含非法数值"
                );
            }
        }
    }
    Ok(())
}

fn parse_abyss_season_names(document: &Value) -> HashMap<u32, String> {
    let mut names = parse_plain_abyss_season_names(document);
    collect_localized_abyss_season_names(document, &mut names);
    names
}

fn parse_plain_abyss_season_names(document: &Value) -> HashMap<u32, String> {
    let Some(object) = document.as_object() else {
        return HashMap::new();
    };
    object
        .iter()
        .filter_map(|(key, value)| {
            let season = key.parse::<u32>().ok()?;
            let name = valid_abyss_season_name(value)?;
            Some((season, name.to_owned()))
        })
        .collect()
}

fn collect_localized_abyss_season_names(document: &Value, names: &mut HashMap<u32, String>) {
    let Some(object) = document.as_object() else {
        return;
    };
    for (key, value) in object {
        if let Some(season_text) = key.strip_circumfix("Abyss_", "_name")
            && let Ok(season) = season_text.parse::<u32>()
            && let Some(name) = valid_abyss_season_name(value)
        {
            names.insert(season, name.to_owned());
        }
        collect_localized_abyss_season_names(value, names);
    }
}

fn valid_abyss_season_name(value: &Value) -> Option<&str> {
    let name = value.as_str()?.trim();
    (!name.is_empty() && !name.contains(',')).then_some(name)
}

fn build_static_index(rows: &HashMap<String, Value>) -> HashMap<String, StaticMonsterInfo> {
    let mut index = HashMap::new();
    for (key, row) in rows {
        let name = row
            .get("Comment")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                row.get("TextName")
                    .and_then(|value| value.get("CultureInvariantString"))
                    .and_then(Value::as_str)
            })
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned);
        let info = StaticMonsterInfo { name };
        for lookup_key in static_monster_lookup_keys(key, row) {
            index.entry(lookup_key).or_insert_with(|| info.clone());
        }
    }
    index
}

fn lookup_static_monster<'a>(
    index: &'a HashMap<String, StaticMonsterInfo>,
    monster_id: &str,
) -> Option<&'a StaticMonsterInfo> {
    monster_lookup_keys(monster_id)
        .into_iter()
        .find_map(|key| index.get(&key))
}

fn static_monster_lookup_keys(key: &str, row: &Value) -> Vec<String> {
    let mut keys = monster_lookup_keys(key);
    if let Some(tags) = row.get("Tags").and_then(Value::as_array) {
        for tag in tags.iter().filter_map(Value::as_str) {
            for key in monster_lookup_keys(tag) {
                push_unique_key(&mut keys, key);
            }
        }
    }
    keys
}

fn monster_lookup_keys(value: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let normalized = normalize_monster_key(value);
    push_unique_key(&mut keys, normalized.clone());
    push_unique_key(&mut keys, normalize_monster_numeric_key(&normalized));
    for suffix in ["_bp", "_bf", "_b"] {
        if let Some(trimmed) = normalized.strip_suffix(suffix) {
            push_unique_key(&mut keys, trimmed.to_owned());
            push_unique_key(&mut keys, normalize_monster_numeric_key(trimmed));
        }
    }
    keys
}

fn push_unique_key(keys: &mut Vec<String>, key: String) {
    if !keys.iter().any(|existing| existing == &key) {
        keys.push(key);
    }
}

fn normalize_monster_key(value: &str) -> String {
    value
        .trim_end_matches("_Abyss")
        .trim_end_matches("_abyss")
        .to_ascii_lowercase()
}

fn normalize_monster_numeric_key(value: &str) -> String {
    value
        .split('_')
        .map(|part| {
            part.parse::<u32>()
                .map(|number| number.to_string())
                .unwrap_or_else(|_| part.to_owned())
        })
        .collect::<Vec<_>>()
        .join("_")
}

fn parse_abyss_group(value: &str) -> Option<u32> {
    value.strip_prefix("Abyss_").and_then(|suffix| {
        if suffix == "Common" {
            Some(0)
        } else {
            suffix.parse::<u32>().ok()
        }
    })
}

fn parse_abyss_route_half(value: &str) -> Option<u32> {
    if value.ends_with("FirstHalf") {
        Some(0)
    } else if value.ends_with("SecondHalf") {
        Some(1)
    } else {
        None
    }
}

fn parse_abyss_attribute_monster_id(attribute_id: &str) -> Option<String> {
    let parts = attribute_id.split('_').collect::<Vec<_>>();
    if parts.len() < 4 || parts.first().copied() != Some("Abyss") {
        return None;
    }
    let monster_start = if parts.get(1).copied() == Some("Common") {
        3
    } else if parts.get(1).is_some_and(|value| is_u32(value))
        && parts.get(2).is_some_and(|value| is_u32(value))
        && parts.get(3).is_some_and(|value| is_u32(value))
        && parts.get(4).is_some_and(|value| is_u32(value))
    {
        5
    } else if parts.get(1).is_some_and(|value| is_u32(value))
        && parts.get(2).is_some_and(|value| is_u32(value))
    {
        3
    } else {
        return None;
    };
    let monster_id = parts.get(monster_start..)?.join("_");
    (!monster_id.is_empty()).then_some(monster_id)
}

fn monster_id_from_class_path(path: &str) -> Option<String> {
    let asset_name = path
        .rsplit_once('/')
        .map(|(_, name)| name)
        .unwrap_or(path)
        .split('.')
        .next()
        .unwrap_or(path);
    let normalized = asset_name
        .trim_end_matches("_C")
        .trim_end_matches("_Abyss")
        .trim_end_matches("_abyss");
    (!normalized.is_empty()).then(|| normalized.to_owned())
}

fn is_u32(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
}

fn string<'a>(row: &'a Value, key: &str) -> Option<&'a str> {
    row.get(key).and_then(Value::as_str)
}

fn u32_value(row: &Value, key: &str) -> Option<u32> {
    row.get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn bool_value(row: &Value, key: &str) -> bool {
    row.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn f64_value(row: &Value, key: &str) -> Option<f64> {
    row.get(key).and_then(Value::as_f64)
}

fn string_array(row: &Value, key: &str) -> Vec<String> {
    row.get(key)
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn monster_stats(row: &Value) -> AbyssMonsterStats {
    let mut raw_props = row
        .as_object()
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| value.as_f64().map(|number| (key.clone(), number)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    raw_props.sort_by(|left, right| left.0.cmp(&right.0));
    AbyssMonsterStats {
        hp_max_base: number(row, "HPMaxBase"),
        raw_props,
    }
}

fn number(row: &Value, key: &str) -> f64 {
    row.get(key).and_then(Value::as_f64).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{
        build_dataset_from_summary, build_static_index, is_supported_abyss_season,
        lookup_static_monster, parse_abyss_attribute_monster_id, parse_abyss_group,
        parse_abyss_route_half, parse_abyss_season_names,
    };
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn supports_released_abyss_season_ten_only() {
        assert!(is_supported_abyss_season(10));
        assert!(!is_supported_abyss_season(11));
    }

    #[test]
    fn parses_summary_abyss_identifiers() {
        assert_eq!(parse_abyss_group("Abyss_Common"), Some(0));
        assert_eq!(parse_abyss_group("Abyss_4"), Some(4));
        assert_eq!(
            parse_abyss_route_half("EAbyssFightStage::FirstHalf"),
            Some(0)
        );
        assert_eq!(
            parse_abyss_route_half("EAbyssFightStage::SecondHalf"),
            Some(1)
        );
        assert_eq!(
            parse_abyss_attribute_monster_id("Abyss_Common_2_mon_35_Blue_BP").as_deref(),
            Some("mon_35_Blue_BP")
        );
        assert_eq!(
            parse_abyss_attribute_monster_id("Abyss_4_6_1_2_mon_03_BP").as_deref(),
            Some("mon_03_BP")
        );
    }

    #[test]
    fn builds_summary_dataset_with_counts_and_stats() {
        let summary_rows = vec![json!({
            "abyss": "Abyss_4",
            "level_id": 2,
            "level_name": "第二站",
            "route": "EAbyssFightStage::FirstHalf",
            "wave": 1,
            "monster_pool_id": "Abyss_4_2_0_1",
            "max_seconds": 600.0,
            "star_thresholds": [
                {"stars": 1, "seconds": 600.0},
                {"stars": 2, "seconds": 420.0},
                {"stars": 3, "seconds": 300.0}
            ],
            "recommended_elements": {
                "first_half": ["光", "咒"],
                "second_half": []
            },
            "monsters": [
                {
                    "name": "罐头锡兵",
                    "class": "/Game/Blueprints/Character/Monster/mon_35/Abyss/mon_35_BP_Blue_Abyss.mon_35_BP_Blue_Abyss_C",
                    "count": 2,
                    "level": 46,
                    "attribute_id": "Abyss_4_2_0_1_mon_35_Blue_BP",
                    "element_types": ["CHARACTER_ELEMENT_TYPE_COSMOS"],
                    "is_boss": false,
                    "spawn_points": ["MonPoint_01", "MonPoint_02"]
                }
            ]
        })];
        let pack_rows = HashMap::from([(
            "Abyss_4_2_0_1_mon_35_Blue_BP".to_owned(),
            json!({
                "HPMaxBase": 1000.0,
                "AttackBase": 50.0
            }),
        )]);
        let static_rows = HashMap::from([(
            "mon_35_BP_Blue_Abyss".to_owned(),
            json!({
                "Comment": "蓝锡兵",
                "Tags": ["mon_35_Blue_BP_Abyss"]
            }),
        )]);
        let static_index = build_static_index(&static_rows);
        let dataset =
            build_dataset_from_summary(&summary_rows, &pack_rows, &static_index, &HashMap::new());
        let floor = dataset.floor(4, 2).expect("summary floor should exist");
        assert_eq!(floor.name.as_deref(), Some("第二站"));
        assert_eq!(floor.monster_count(), 2);
        assert_eq!(floor.wave_count(), 1);

        let monster = floor
            .monsters
            .first()
            .expect("summary monster should exist");
        assert_eq!(monster.count, 2);
        assert_eq!(monster.level, Some(46));
        assert_eq!(monster.half, Some(0));
        assert_eq!(monster.wave, Some(1));
        assert_eq!(monster.monster_id, "mon_35_Blue_BP");
        assert_eq!(monster.stats.hp_max_base, 1000.0);

        assert_eq!(floor.max_seconds, Some(600.0));
        assert_eq!(
            floor.star_thresholds,
            vec![
                super::AbyssStarThreshold {
                    stars: 1,
                    seconds: 600.0
                },
                super::AbyssStarThreshold {
                    stars: 2,
                    seconds: 420.0
                },
                super::AbyssStarThreshold {
                    stars: 3,
                    seconds: 300.0
                },
            ]
        );
        assert_eq!(
            floor.recommended_elements,
            super::AbyssRecommendedElements {
                first_half: vec!["光".to_owned(), "咒".to_owned()],
                second_half: Vec::new(),
            }
        );
    }

    #[test]
    fn static_lookup_matches_tags_and_numeric_padding() {
        let rows = HashMap::from([(
            "mon_016_BP_Abyss".to_owned(),
            json!({
                "Comment": "贩卖机",
                "TextName": {"CultureInvariantString": null},
                "Tags": ["mon_16_BP_Abyss"]
            }),
        )]);
        let index = build_static_index(&rows);

        assert_eq!(
            lookup_static_monster(&index, "mon_16_BP").and_then(|info| info.name.as_deref()),
            Some("贩卖机")
        );
    }

    #[test]
    fn static_lookup_matches_bp_and_bf_suffix_variants() {
        let rows = HashMap::from([(
            "mon_35_BP_Red_Abyss".to_owned(),
            json!({
                "Comment": "红锡兵(近战)",
                "Tags": ["mon_35_Red_BP_Abyss"]
            }),
        )]);
        let index = build_static_index(&rows);

        assert_eq!(
            lookup_static_monster(&index, "mon_35_Red_BF").and_then(|info| info.name.as_deref()),
            Some("红锡兵(近战)")
        );
    }

    #[test]
    fn parses_localized_abyss_season_names() {
        let names = parse_abyss_season_names(&json!({
            "Abyss_4_name": "晦冥环线",
            "Abyss_5_name": "ST_AbyssBattle,Abyss_5_name",
            "Buff_Abyss_Phase_004_name": "无星之夜"
        }));

        assert_eq!(names.get(&4).map(String::as_str), Some("晦冥环线"));
        assert!(!names.contains_key(&5));
    }

    #[test]
    fn parses_plain_abyss_season_names_resource() {
        let names = parse_abyss_season_names(&json!({
            "4": "晦冥环线",
            "5": "晦冥环线"
        }));

        assert_eq!(names.get(&4).map(String::as_str), Some("晦冥环线"));
        assert_eq!(names.get(&5).map(String::as_str), Some("晦冥环线"));
    }
}
