use crate::model::Hit;
use crate::object_state::{
    ObjectDescriptor, ObjectHandleKind, ObjectStateStore, is_targetish_path,
};
use crate::resource_index::ResourceIndex;
use crate::ue_bitstream::PathCandidate;

const HP_MATCH_TOLERANCE_ABSOLUTE: f64 = 2.0;
const HP_MATCH_TOLERANCE_RATIO: f64 = 0.002;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TargetConfidence {
    #[default]
    Unknown,
    Possible,
    Probable,
    Confirmed,
}

impl TargetConfidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Possible => "possible",
            Self::Probable => "probable",
            Self::Confirmed => "confirmed",
        }
    }

    pub fn rank(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Possible => 1,
            Self::Probable => 2,
            Self::Confirmed => 3,
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TargetResolutionSummary {
    pub target_id: Option<String>,
    pub target_name: Option<String>,
    pub target_context: Vec<String>,
    pub score: i32,
    pub confidence: TargetConfidence,
    pub direct_hp_evidence: bool,
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
            let mut score = 25;
            let mut reasons = vec![format!("path_candidate:{}", path.value)];
            if path.value.starts_with("/Game/") {
                score += 15;
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

    #[allow(dead_code)]
    pub fn apply_to_hit(
        &self,
        hit: &mut Hit,
        store: &ObjectStateStore,
        packet_paths: &[PathCandidate],
        resources: &ResourceIndex,
    ) -> Vec<TargetCandidate> {
        let candidates = self.resolve_for_hit(hit, store, packet_paths, resources);
        apply_candidates_to_hit(hit, &candidates);
        candidates
    }

    pub fn apply_to_hit_with_summary(
        &self,
        hit: &mut Hit,
        store: &ObjectStateStore,
        packet_paths: &[PathCandidate],
        resources: &ResourceIndex,
    ) -> TargetResolutionSummary {
        let candidates = self.resolve_for_hit(hit, store, packet_paths, resources);
        apply_candidates_to_hit(hit, &candidates);
        TargetResolutionSummary::from_hit_and_candidates(hit, &candidates)
    }
}

impl TargetResolutionSummary {
    pub fn from_hit_and_candidates(hit: &Hit, candidates: &[TargetCandidate]) -> Self {
        let Some(top) = candidates.first() else {
            return Self {
                target_id: hit.target_id.clone(),
                target_name: hit.target_name.clone(),
                target_context: hit.target_context.clone(),
                score: 0,
                confidence: TargetConfidence::Unknown,
                direct_hp_evidence: false,
            };
        };
        Self {
            target_id: hit.target_id.clone(),
            target_name: hit.target_name.clone(),
            target_context: hit.target_context.clone(),
            score: top.score,
            confidence: top.confidence,
            direct_hp_evidence: candidate_has_direct_hp_evidence(top),
        }
    }
}

fn apply_candidates_to_hit(hit: &mut Hit, candidates: &[TargetCandidate]) {
    let Some(top) = candidates.first() else {
        hit.target_context
            .push("confidence=unknown score=0 reason=no_target_candidate".to_owned());
        return;
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
    ) && top.handle_kind != ObjectHandleKind::PathOnly
    {
        hit.target_name = top.target_name.clone();
    }
    if candidates.len() > 1 {
        hit.target_context
            .push(format!("candidate_count={}", candidates.len()));
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
    let path_only = object.handle_kind == ObjectHandleKind::PathOnly;
    let target_path = object
        .object_path
        .clone()
        .or_else(|| object.class_path.clone());
    if let Some(path) = &target_path
        && is_targetish_path(path)
    {
        score += if path_only { 25 } else { 30 };
        reasons.push(format!("near_object_path:{path}"));
        if path.starts_with("/Game/") {
            score += if path_only { 15 } else { 40 };
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
            direct_hp_match = true;
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
    if !path_only && high_conf_targets == 1 && target_path.as_deref().is_some_and(is_targetish_path)
    {
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
    candidates.iter().any(candidate_has_direct_hp_evidence)
}

fn candidate_has_direct_hp_evidence(candidate: &TargetCandidate) -> bool {
    candidate.reasons.iter().any(|reason| {
        reason.starts_with("hp_guid_timeline_match")
            || reason.starts_with("boss_hp_delta_match")
            || (candidate.handle_kind == ObjectHandleKind::AttributeGuid
                && reason == "last_hp_close_to_hit_after")
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

    #[test]
    fn path_only_candidate_does_not_fill_target_name() {
        let path = PathCandidate {
            value: "/Game/Blueprints/Character/Monster/boss_07/BP_Boss_07.BP_Boss_07_C".to_owned(),
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
        assert_eq!(candidates[0].handle_kind, ObjectHandleKind::PathOnly);
        assert_eq!(candidates[0].confidence, TargetConfidence::Possible);
        assert!(hit.target_name.is_none());
        assert!(
            hit.target_context
                .iter()
                .any(|item| item.contains("possible"))
        );
    }

    #[test]
    fn direct_hp_match_avoids_conflict_penalty() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::default();
        let guid = [10_u8; 16];
        store.observe_hp_guid_update(9.8, guid, 1000.0, None, "hp=1000".to_owned());
        store.observe_hp_guid_update(10.1, guid, 900.0, None, "hp=900".to_owned());
        store.observe_path_candidate(
            10.0,
            &PathCandidate {
                value: "/Game/Monster/OtherBoss".to_owned(),
                byte_offset: 0,
                bit_shift: 0,
                score: 240,
            },
            &resources,
        );

        let candidates = TargetResolver.resolve_for_hit(&hit(), &store, &[], &resources);
        let direct = candidates
            .iter()
            .find(|candidate| candidate.handle_kind == ObjectHandleKind::AttributeGuid)
            .expect("attribute candidate should exist");
        assert!(
            direct
                .reasons
                .iter()
                .any(|reason| reason.starts_with("hp_guid_timeline_match"))
        );
        assert!(
            !direct
                .reasons
                .iter()
                .any(|reason| reason == "conflict:multiple_candidates")
        );
    }

    #[test]
    fn boss_hp_delta_counts_as_direct_hp_evidence() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::default();
        let guid = [11_u8; 16];
        store.observe_hp_guid_update(9.8, guid, 1000.0, None, "hp=1000".to_owned());
        store.observe_hp_guid_update(10.1, guid, 900.0, None, "hp=900".to_owned());
        let mut hit = hit();
        hit.target_hp_before = 0.0;
        hit.target_hp_after = 0.0;
        store.observe_path_candidate(
            10.0,
            &PathCandidate {
                value: "/Game/Monster/PathOnlyBoss".to_owned(),
                byte_offset: 0,
                bit_shift: 0,
                score: 240,
            },
            &resources,
        );

        let candidates = TargetResolver.resolve_for_hit(&hit, &store, &[], &resources);
        let direct = candidates
            .iter()
            .find(|candidate| candidate.handle_kind == ObjectHandleKind::AttributeGuid)
            .expect("attribute candidate should exist");
        assert!(
            direct
                .reasons
                .iter()
                .any(|reason| reason.starts_with("boss_hp_delta_match"))
        );
        assert!(
            !direct
                .reasons
                .iter()
                .any(|reason| reason == "conflict:multiple_candidates")
        );
    }

    #[test]
    fn multiple_path_only_candidates_remain_possible() {
        let resources = ResourceIndex::default();
        let paths = [
            PathCandidate {
                value: "/Game/Monster/BossAlpha".to_owned(),
                byte_offset: 0,
                bit_shift: 0,
                score: 240,
            },
            PathCandidate {
                value: "/Game/Monster/BossBeta".to_owned(),
                byte_offset: 8,
                bit_shift: 0,
                score: 240,
            },
        ];
        let candidates = TargetResolver.resolve_for_hit(
            &hit(),
            &ObjectStateStore::default(),
            &paths,
            &resources,
        );
        assert!(candidates.iter().all(|candidate| {
            candidate.handle_kind == ObjectHandleKind::PathOnly
                && candidate.confidence.rank() <= TargetConfidence::Possible.rank()
        }));
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.target_name.is_some())
        );
    }
}
