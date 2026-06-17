use crate::model::Hit;
use crate::object_state::{
    ObjectDescriptor, ObjectHandleKind, ObjectStateStore, is_targetish_path,
};
use crate::resource_index::ResourceIndex;
use crate::ue_bitstream::PathCandidate;

const HP_MATCH_TOLERANCE_ABSOLUTE: f64 = 2.0;
const HP_MATCH_TOLERANCE_RATIO: f64 = 0.002;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetConfidence {
    Confirmed,
    Probable,
    Possible,
    Unknown,
}

impl TargetConfidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Probable => "probable",
            Self::Possible => "possible",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug)]
pub struct TargetCandidate {
    pub handle: String,
    pub handle_kind: ObjectHandleKind,
    pub target_name: Option<String>,
    pub target_path: Option<String>,
    pub score: i32,
    pub confidence: TargetConfidence,
    pub reasons: Vec<String>,
}

#[derive(Default)]
pub struct TargetResolver;

impl TargetResolver {
    pub fn resolve_for_hit(
        &self,
        hit: &Hit,
        store: &ObjectStateStore,
        packet_paths: &[PathCandidate],
        resources: &ResourceIndex,
    ) -> Vec<TargetCandidate> {
        let mut candidates = store
            .candidates_for_damage(hit.timestamp)
            .into_iter()
            .map(|object| score_object(hit, object, store, resources))
            .collect::<Vec<_>>();
        for path in packet_paths
            .iter()
            .filter(|path| is_targetish_path(&path.value))
        {
            if candidates.iter().any(|candidate| {
                candidate.target_path.as_deref() == Some(path.value.as_str())
                    || candidate.handle == path.value
            }) {
                continue;
            }
            let mut score = 30;
            let mut reasons = vec![format!("path_candidate:{}", path.value)];
            if path.value.starts_with("/Game/") {
                score += 40;
                reasons.push("game_path_target_keyword".to_owned());
            }
            let confidence = confidence_for_score(score);
            candidates.push(TargetCandidate {
                handle: path.value.clone(),
                handle_kind: ObjectHandleKind::PathOnly,
                target_name: resources.display_name_for_path(&path.value),
                target_path: Some(path.value.clone()),
                score,
                confidence,
                reasons,
            });
        }
        if candidates.len() > 1 {
            let strong = candidates
                .iter()
                .filter(|candidate| candidate.score >= 35)
                .count();
            if strong > 1 && !has_direct_hp_match(&candidates) {
                for candidate in &mut candidates {
                    candidate.score -= 20;
                    candidate
                        .reasons
                        .push("conflict:multiple_candidates".to_owned());
                    candidate.confidence = confidence_for_score(candidate.score);
                }
            }
        }
        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.score));
        candidates
    }

    pub fn apply_to_hit(
        &self,
        hit: &mut Hit,
        store: &ObjectStateStore,
        packet_paths: &[PathCandidate],
        resources: &ResourceIndex,
    ) -> Vec<TargetCandidate> {
        let candidates = self.resolve_for_hit(hit, store, packet_paths, resources);
        let Some(top) = candidates.first() else {
            hit.target_context
                .push("confidence=unknown score=0 reason=no_target_candidate".to_owned());
            return candidates;
        };
        hit.target_context.push(format!(
            "confidence={} score={}",
            top.confidence.as_str(),
            top.score
        ));
        hit.target_context.extend(
            top.reasons
                .iter()
                .take(6)
                .map(|reason| format!("reason={reason}")),
        );
        if let Some(path) = &top.target_path {
            hit.target_context.push(format!("target_path={path}"));
        }
        if top.confidence != TargetConfidence::Unknown {
            hit.target_id = Some(format!("{}:{}", top.handle_kind.label(), top.handle));
        }
        if matches!(
            top.confidence,
            TargetConfidence::Confirmed | TargetConfidence::Probable
        ) {
            hit.target_name = top.target_name.clone();
        }
        if candidates.len() > 1 {
            hit.target_context
                .push(format!("candidate_count={}", candidates.len()));
        }
        candidates
    }
}

fn score_object(
    hit: &Hit,
    object: &ObjectDescriptor,
    store: &ObjectStateStore,
    resources: &ResourceIndex,
) -> TargetCandidate {
    let mut score = 0;
    let mut reasons = Vec::new();
    let target_path = object
        .object_path
        .clone()
        .or_else(|| object.class_path.clone());
    if let Some(path) = &target_path
        && is_targetish_path(path)
    {
        score += 30;
        reasons.push(format!("near_object_path:{path}"));
        if path.starts_with("/Game/") {
            score += 40;
            reasons.push("game_path_target_keyword".to_owned());
        }
    }

    let mut direct_hp_match = false;
    for observation_pair in object
        .hp_history
        .as_slices()
        .0
        .windows(2)
        .chain(object.hp_history.as_slices().1.windows(2))
    {
        let previous = &observation_pair[0];
        let current = &observation_pair[1];
        let delta = previous.current - current.current;
        let time_delta = (current.timestamp - hit.timestamp).abs();
        if time_delta <= 1.0 && nearly_equal(delta, hit.damage, hit.damage) {
            score += 50;
            reasons.push(format!("boss_hp_delta_match:{delta:.0}"));
            let time_bonus = ((1.0 - time_delta).max(0.0) * 20.0).round() as i32;
            if time_bonus > 0 {
                score += time_bonus;
                reasons.push(format!("hp_update_time_bonus:{time_bonus}"));
            }
        }
        if time_delta <= 1.0
            && nearly_equal(previous.current, hit.target_hp_before, hit.target_hp_before)
            && nearly_equal(current.current, hit.target_hp_after, hit.target_hp_after)
        {
            score += 50;
            direct_hp_match = true;
            reasons.push(format!(
                "hp_guid_timeline_match:{}->{}",
                previous.evidence, current.evidence
            ));
        }
    }

    if !direct_hp_match
        && let Some(current_hp) = object.hp_current
        && (current_hp - hit.target_hp_after).abs() <= hp_tolerance(hit.target_hp_before)
    {
        score += 20;
        reasons.push("last_hp_close_to_hit_after".to_owned());
    }
    if hit.target_max_hp > 500_000.0 && object.hp_current.is_some() {
        score += 5;
        reasons.push("target_max_hp_only_weak".to_owned());
    }

    let high_conf_targets = store
        .objects_near_time(hit.timestamp, 1.0)
        .into_iter()
        .filter(|candidate| {
            candidate.confidence >= 0.65
                && candidate
                    .object_path
                    .as_deref()
                    .or(candidate.class_path.as_deref())
                    .is_some_and(is_targetish_path)
        })
        .count();
    if high_conf_targets == 1 && target_path.as_deref().is_some_and(is_targetish_path) {
        score += 25;
        reasons.push("single_high_confidence_target_window".to_owned());
    }

    let target_name = object.display_name.clone().or_else(|| {
        target_path
            .as_deref()
            .and_then(|path| resources.display_name_for_path(path))
    });
    let confidence = confidence_for_score(score);
    TargetCandidate {
        handle: object.handle.clone(),
        handle_kind: object.handle_kind.clone(),
        target_name,
        target_path,
        score,
        confidence,
        reasons,
    }
}

fn has_direct_hp_match(candidates: &[TargetCandidate]) -> bool {
    candidates.iter().any(|candidate| {
        candidate
            .reasons
            .iter()
            .any(|reason| reason.starts_with("hp_guid_timeline_match"))
    })
}

fn confidence_for_score(score: i32) -> TargetConfidence {
    if score >= 90 {
        TargetConfidence::Confirmed
    } else if score >= 60 {
        TargetConfidence::Probable
    } else if score >= 35 {
        TargetConfidence::Possible
    } else {
        TargetConfidence::Unknown
    }
}

fn nearly_equal(left: f64, right: f64, scale: f64) -> bool {
    (left - right).abs() <= hp_tolerance(scale)
}

fn hp_tolerance(scale: f64) -> f64 {
    HP_MATCH_TOLERANCE_ABSOLUTE.max(scale.abs() * HP_MATCH_TOLERANCE_RATIO)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Hit;
    use crate::object_state::ObjectStateStore;

    fn hit() -> Hit {
        Hit {
            timestamp: 10.0,
            char_id: 1,
            char_name: "test".to_owned(),
            char_known: true,
            damage: 100.0,
            byte_offset: 0,
            bit_shift: 0,
            char_source: "test".to_owned(),
            direction: "outgoing".to_owned(),
            target_hp_before: 1000.0,
            target_hp_after: 900.0,
            target_max_hp: 1000.0,
            target_hp_percent: 90.0,
            target_id: None,
            target_name: None,
            target_context: Vec::new(),
            gameplay_effect_index: None,
            gameplay_effect_name: None,
            ability_name: None,
            damage_name: None,
            attack_type: None,
        }
    }

    #[test]
    fn single_boss_hp_delta_generates_probable_or_confirmed_target() {
        let mut store = ObjectStateStore::default();
        let guid = [7_u8; 16];
        store.observe_hp_guid_update(9.8, guid, 1000.0, None, "hp=1000".to_owned());
        store.observe_hp_guid_update(10.1, guid, 900.0, None, "hp=900".to_owned());
        let resolver = TargetResolver;
        let candidates = resolver.resolve_for_hit(&hit(), &store, &[], &ResourceIndex::default());
        assert!(candidates[0].score >= 60);
        assert!(
            candidates[0]
                .reasons
                .iter()
                .any(|reason| reason.contains("boss_hp_delta_match"))
        );
    }

    #[test]
    fn multiple_candidates_without_direct_evidence_are_not_confirmed() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::default();
        store.observe_path_candidate(
            10.0,
            &PathCandidate {
                value: "/Game/Monster/A".to_owned(),
                byte_offset: 0,
                bit_shift: 0,
                score: 240,
            },
            &resources,
        );
        store.observe_path_candidate(
            10.0,
            &PathCandidate {
                value: "/Game/Monster/B".to_owned(),
                byte_offset: 8,
                bit_shift: 0,
                score: 240,
            },
            &resources,
        );
        let candidates = TargetResolver.resolve_for_hit(&hit(), &store, &[], &resources);
        assert_ne!(candidates[0].confidence, TargetConfidence::Confirmed);
    }

    #[test]
    fn only_target_max_hp_stays_possible_or_lower() {
        let mut store = ObjectStateStore::default();
        let guid = [9_u8; 16];
        store.observe_hp_guid_update(10.0, guid, 900.0, None, "hp=900".to_owned());
        let mut hit = hit();
        hit.target_max_hp = 1_000_000.0;
        let candidates =
            TargetResolver.resolve_for_hit(&hit, &store, &[], &ResourceIndex::default());
        assert!(candidates[0].score < 60);
    }

    #[test]
    fn unknown_does_not_fill_target_name() {
        let store = ObjectStateStore::default();
        let mut hit = hit();
        let candidates =
            TargetResolver.apply_to_hit(&mut hit, &store, &[], &ResourceIndex::default());
        assert!(candidates.is_empty());
        assert!(hit.target_name.is_none());
        assert!(hit.target_context[0].contains("unknown"));
    }

    #[test]
    fn reasons_include_readable_evidence() {
        let path = PathCandidate {
            value: "/Game/Blueprints/Character/Monster/boss_07/BP.BP_C".to_owned(),
            byte_offset: 1,
            bit_shift: 0,
            score: 240,
        };
        let mut hit = hit();
        let candidates = TargetResolver.apply_to_hit(
            &mut hit,
            &ObjectStateStore::default(),
            &[path],
            &ResourceIndex::default(),
        );
        assert!(
            candidates[0]
                .reasons
                .iter()
                .any(|reason| reason.contains("/Game/"))
        );
        assert!(
            hit.target_context
                .iter()
                .any(|item| item.contains("reason="))
        );
    }
}
