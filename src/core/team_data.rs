//! Frontend-neutral import/export helpers for the compact team DPS exchange
//! format. UI adapters own file pickers and download affordances; validation
//! and projection stay here so desktop and CLI boundaries share one contract.

use crate::engine::model::{
    CharacterStats, CombatState, TEAM_DPS_EXPORT_VERSION, TEAM_DPS_MAX_MEMBERS, TeamDps,
    TeamDpsExport, TeamDpsMember,
};

pub const MAX_TEAM_DPS_EXPORT_BYTES: usize = 1024 * 1024;

pub fn export_team_data(
    state: &CombatState,
    subtract_time_stop: bool,
    separate_reaction_damage: bool,
    imported_upper: Option<TeamDps>,
    imported_lower: Option<TeamDps>,
) -> Option<TeamDpsExport> {
    let stats = state
        .stats
        .values()
        .map(|row| row.for_reaction_damage_policy(separate_reaction_damage))
        .collect::<Vec<_>>();
    let single = snapshot_team_from_stats(
        state.dps_with_time_stop(subtract_time_stop),
        state.duration_with_time_stop(subtract_time_stop),
        stats.iter(),
    );
    let upper = snapshot_party_team(
        &state.abyss.first_half,
        subtract_time_stop,
        separate_reaction_damage,
    )
    .or(imported_upper);
    let lower = snapshot_party_team(
        &state.abyss.second_half,
        subtract_time_stop,
        separate_reaction_damage,
    )
    .or(imported_lower);
    (single.is_some() || upper.is_some() || lower.is_some()).then_some(TeamDpsExport {
        version: TEAM_DPS_EXPORT_VERSION,
        single,
        upper,
        lower,
    })
}

pub fn parse_team_data(json: &str) -> Result<TeamDpsExport, &'static str> {
    if json.len() > MAX_TEAM_DPS_EXPORT_BYTES {
        return Err("team data exceeds the size limit");
    }
    let export: TeamDpsExport =
        serde_json::from_str(json).map_err(|_| "team data is not valid JSON")?;
    if export.version != TEAM_DPS_EXPORT_VERSION {
        return Err("team data version is unsupported");
    }
    if export.single.is_none() && export.upper.is_none() && export.lower.is_none() {
        return Err("team data contains no team");
    }
    for team in [&export.single, &export.upper, &export.lower]
        .into_iter()
        .flatten()
    {
        validate_team(team)?;
    }
    Ok(export)
}

fn validate_team(team: &TeamDps) -> Result<(), &'static str> {
    if !team.dps.is_finite() || team.dps <= 0.0 {
        return Err("team DPS must be finite and positive");
    }
    if team.members.len() > TEAM_DPS_MAX_MEMBERS {
        return Err("team data contains too many members");
    }
    if team
        .members
        .iter()
        .any(|member| member.id == 0 || !member.dps.is_finite() || member.dps < 0.0)
    {
        return Err("team member data is invalid");
    }
    Ok(())
}

fn snapshot_party_team(
    party: &crate::engine::model::PartyCombatState,
    subtract_time_stop: bool,
    separate_reaction_damage: bool,
) -> Option<TeamDps> {
    let stats = party
        .stats
        .values()
        .map(|row| row.for_reaction_damage_policy(separate_reaction_damage))
        .collect::<Vec<_>>();
    snapshot_team_from_stats(
        party.dps_with_time_stop(subtract_time_stop),
        party.duration_with_time_stop(subtract_time_stop),
        stats.iter(),
    )
}

fn snapshot_team_from_stats<'a>(
    dps: f64,
    duration: f64,
    stats: impl IntoIterator<Item = &'a CharacterStats>,
) -> Option<TeamDps> {
    if !dps.is_finite() || dps <= 0.0 {
        return None;
    }
    let shared_duration = duration.max(1.0);
    let mut members: Vec<&CharacterStats> = stats
        .into_iter()
        .filter(|stats| stats.char_id != 0 && stats.char_id < 900_000 && stats.damage > 0.0)
        .collect();
    members.sort_by(|left, right| {
        right
            .damage
            .total_cmp(&left.damage)
            .then_with(|| left.char_id.cmp(&right.char_id))
    });
    members.truncate(TEAM_DPS_MAX_MEMBERS);
    Some(TeamDps {
        dps,
        members: members
            .into_iter()
            .map(|stats| TeamDpsMember {
                id: stats.char_id,
                dps: stats.damage / shared_duration,
                name: stats.name.clone(),
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_team_data_is_bounded_and_versioned() {
        let valid = r#"{"version":1,"single":{"dps":100,"members":[{"id":1,"dps":100}]}}"#;
        assert_eq!(
            parse_team_data(valid)
                .expect("valid team data")
                .single
                .expect("single team")
                .dps,
            100.0
        );
        assert!(parse_team_data(r#"{"version":2,"single":{"dps":100}}"#).is_err());
        assert!(parse_team_data(r#"{"version":1}"#).is_err());
        assert!(parse_team_data(r#"{"version":1,"single":{"dps":-1}}"#).is_err());
    }
}
