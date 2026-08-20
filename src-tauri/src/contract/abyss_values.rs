use std::collections::BTreeMap;

use nte_dps_tool::engine::{
    abyss_data::{
        AbyssFloor, AbyssMonsterEntry, AbyssRecommendedElements, AbyssSeason, AbyssStarThreshold,
    },
    model::{TeamDps, TeamDpsMember},
};
use nte_dps_tool::storage::abyss_remote::LoadedAbyssDataset;
use serde::Serialize;

pub(crate) const ABYSS_VALUES_CONTRACT_VERSION: u32 = 3;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AbyssValuesSnapshot {
    pub contract_version: u32,
    pub data_version: String,
    pub data_updated_at: String,
    pub data_stale: bool,
    pub season_count: usize,
    pub floor_count: usize,
    pub monster_count: u32,
    pub seasons: Vec<AbyssSeasonSnapshot>,
    pub teams: AbyssPredictionTeamsSnapshot,
    pub current_team_available: AbyssCurrentTeamAvailabilitySnapshot,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AbyssSeasonSnapshot {
    pub season: u32,
    pub name: Option<String>,
    pub floors: Vec<AbyssFloorSnapshot>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AbyssFloorSnapshot {
    pub season: u32,
    pub season_name: Option<String>,
    pub floor: u32,
    pub name: Option<String>,
    pub monster_count: u32,
    pub wave_count: usize,
    pub max_seconds: Option<f64>,
    pub star_thresholds: Vec<AbyssStarThresholdSnapshot>,
    pub recommended_elements: AbyssRecommendedElementsSnapshot,
    pub monsters: Vec<AbyssMonsterSnapshot>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AbyssStarThresholdSnapshot {
    pub stars: u32,
    pub seconds: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AbyssRecommendedElementsSnapshot {
    pub first_half: Vec<String>,
    pub second_half: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AbyssMonsterSnapshot {
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
    pub hp_max_base: f64,
    pub raw_props: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AbyssPredictionTeamsSnapshot {
    pub upper: Option<AbyssTeamSnapshot>,
    pub lower: Option<AbyssTeamSnapshot>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AbyssCurrentTeamAvailabilitySnapshot {
    pub upper: bool,
    pub lower: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AbyssTeamSnapshot {
    pub dps: f64,
    pub members: Vec<AbyssTeamMemberSnapshot>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AbyssTeamMemberSnapshot {
    pub id: u32,
    pub dps: f64,
    pub name: String,
}

impl AbyssValuesSnapshot {
    pub(crate) fn new(
        loaded: LoadedAbyssDataset,
        teams: (Option<TeamDps>, Option<TeamDps>),
        current_team_available: [bool; 2],
    ) -> Self {
        let LoadedAbyssDataset {
            dataset,
            data_version,
            updated_at,
            stale,
        } = loaded;
        let season_count = dataset.seasons.len();
        let floor_count = dataset
            .seasons
            .iter()
            .map(|season| season.floors.len())
            .sum();
        let monster_count = dataset
            .seasons
            .iter()
            .flat_map(|season| &season.floors)
            .map(AbyssFloor::monster_count)
            .sum();
        Self {
            contract_version: ABYSS_VALUES_CONTRACT_VERSION,
            data_version,
            data_updated_at: updated_at,
            data_stale: stale,
            season_count,
            floor_count,
            monster_count,
            seasons: dataset
                .seasons
                .into_iter()
                .map(AbyssSeasonSnapshot::from)
                .collect(),
            teams: AbyssPredictionTeamsSnapshot::from(teams),
            current_team_available: AbyssCurrentTeamAvailabilitySnapshot {
                upper: current_team_available[0],
                lower: current_team_available[1],
            },
        }
    }
}

impl From<AbyssSeason> for AbyssSeasonSnapshot {
    fn from(season: AbyssSeason) -> Self {
        Self {
            season: season.season,
            name: season.name,
            floors: season
                .floors
                .into_iter()
                .map(AbyssFloorSnapshot::from)
                .collect(),
        }
    }
}

impl From<AbyssFloor> for AbyssFloorSnapshot {
    fn from(floor: AbyssFloor) -> Self {
        let monster_count = floor.monster_count();
        let wave_count = floor.wave_count();
        Self {
            season: floor.season,
            season_name: floor.season_name,
            floor: floor.floor,
            name: floor.name,
            monster_count,
            wave_count,
            max_seconds: floor.max_seconds,
            star_thresholds: floor
                .star_thresholds
                .into_iter()
                .map(AbyssStarThresholdSnapshot::from)
                .collect(),
            recommended_elements: floor.recommended_elements.into(),
            monsters: floor
                .monsters
                .into_iter()
                .map(AbyssMonsterSnapshot::from)
                .collect(),
        }
    }
}

impl From<AbyssStarThreshold> for AbyssStarThresholdSnapshot {
    fn from(threshold: AbyssStarThreshold) -> Self {
        Self {
            stars: threshold.stars,
            seconds: threshold.seconds,
        }
    }
}

impl From<AbyssRecommendedElements> for AbyssRecommendedElementsSnapshot {
    fn from(elements: AbyssRecommendedElements) -> Self {
        Self {
            first_half: elements.first_half,
            second_half: elements.second_half,
        }
    }
}

impl From<AbyssMonsterEntry> for AbyssMonsterSnapshot {
    fn from(monster: AbyssMonsterEntry) -> Self {
        Self {
            pack_id: monster.pack_id,
            attribute_id: monster.attribute_id,
            monster_pool_id: monster.monster_pool_id,
            monster_id: monster.monster_id,
            name: monster.name,
            count: monster.count,
            level: monster.level,
            half: monster.half,
            wave: monster.wave,
            is_boss: monster.is_boss,
            hp_max_base: monster.stats.hp_max_base,
            raw_props: monster.stats.raw_props.into_iter().collect(),
        }
    }
}

impl From<(Option<TeamDps>, Option<TeamDps>)> for AbyssPredictionTeamsSnapshot {
    fn from(teams: (Option<TeamDps>, Option<TeamDps>)) -> Self {
        Self {
            upper: teams.0.map(AbyssTeamSnapshot::from),
            lower: teams.1.map(AbyssTeamSnapshot::from),
        }
    }
}

impl From<TeamDps> for AbyssTeamSnapshot {
    fn from(team: TeamDps) -> Self {
        Self {
            dps: team.dps,
            members: team
                .members
                .into_iter()
                .map(AbyssTeamMemberSnapshot::from)
                .collect(),
        }
    }
}

impl From<TeamDpsMember> for AbyssTeamMemberSnapshot {
    fn from(member: TeamDpsMember) -> Self {
        Self {
            id: member.id,
            dps: member.dps,
            name: member.name,
        }
    }
}
