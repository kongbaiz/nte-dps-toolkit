use std::collections::{HashMap, VecDeque};

use crate::resource_index::ResourceIndex;
use crate::ue_bitstream::PathCandidate;

const MAX_OBJECTS: usize = 512;
const MAX_EVIDENCE_PER_OBJECT: usize = 8;
const MAX_HP_HISTORY_PER_OBJECT: usize = 32;
const OBJECT_TTL_SECONDS: f64 = 20.0;
const ATTRIBUTE_PATH_LINK_WINDOW_SECONDS: f64 = 6.0;
const HP_HISTORY_DAMAGE_WINDOW_SECONDS: f64 = 1.0;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
#[allow(dead_code)]
pub enum ObjectHandleKind {
    RuntimeInstance,
    NetGuidCandidate,
    NetRefHandleCandidate,
    AttributeGuid,
    PathOnly,
    Unknown,
}

impl ObjectHandleKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::RuntimeInstance => "RuntimeInstance",
            Self::NetGuidCandidate => "NetGuidCandidate",
            Self::NetRefHandleCandidate => "NetRefHandleCandidate",
            Self::AttributeGuid => "AttributeGuid",
            Self::PathOnly => "PathOnly",
            Self::Unknown => "Unknown",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct HpObservation {
    pub timestamp: f64,
    pub current: f64,
    pub max: Option<f64>,
    pub evidence: String,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ObjectDescriptor {
    pub handle_kind: ObjectHandleKind,
    pub handle: String,
    pub class_path: Option<String>,
    pub object_path: Option<String>,
    pub display_name: Option<String>,
    pub table_resolved_name: bool,
    pub owner_handle: Option<String>,
    pub actor_handle: Option<String>,
    pub component_handle: Option<String>,
    pub hp_current: Option<f64>,
    pub hp_max: Option<f64>,
    pub first_seen_at: f64,
    pub last_seen_at: f64,
    pub evidence: Vec<String>,
    pub confidence: f32,
    pub hp_history: VecDeque<HpObservation>,
}

#[derive(Clone, Debug, Default)]
pub struct ObjectStateStore {
    objects: HashMap<String, ObjectDescriptor>,
}

impl ObjectStateStore {
    pub fn observe_path_candidate(
        &mut self,
        timestamp: f64,
        candidate: &PathCandidate,
        resources: &ResourceIndex,
    ) -> String {
        let key = object_key(&ObjectHandleKind::PathOnly, &candidate.value);
        let target_path = resources
            .canonical_target_path_for_path(&candidate.value)
            .unwrap_or_else(|| candidate.value.clone());
        let has_resolved_name = resources.resolved_name_for_path(&target_path).is_some()
            || resources.resolved_name_for_path(&candidate.value).is_some();
        let display_name = resources
            .display_name_for_path(&target_path)
            .or_else(|| resources.display_name_for_path(&candidate.value));
        let confidence =
            path_candidate_confidence(candidate.score, &target_path, has_resolved_name);
        let descriptor = self
            .objects
            .entry(key.clone())
            .or_insert_with(|| ObjectDescriptor {
                handle_kind: ObjectHandleKind::PathOnly,
                handle: candidate.value.clone(),
                class_path: Some(target_path.clone()).filter(|value| value.contains("/Game/")),
                object_path: Some(target_path.clone()),
                display_name: display_name.clone(),
                table_resolved_name: has_resolved_name,
                owner_handle: None,
                actor_handle: None,
                component_handle: None,
                hp_current: None,
                hp_max: None,
                first_seen_at: timestamp,
                last_seen_at: timestamp,
                evidence: Vec::new(),
                confidence,
                hp_history: VecDeque::new(),
            });
        descriptor.last_seen_at = timestamp;
        descriptor.confidence = descriptor.confidence.max(confidence);
        descriptor.object_path = Some(target_path.clone());
        descriptor.table_resolved_name |= has_resolved_name;
        if target_path.contains("/Game/") {
            descriptor.class_path = Some(target_path.clone());
        }
        descriptor.display_name = descriptor.display_name.clone().or(display_name);
        push_unique_evidence(
            &mut descriptor.evidence,
            format!(
                "path_candidate:{}@{}:{}",
                candidate.value, candidate.byte_offset, candidate.bit_shift
            ),
        );
        self.link_path_to_single_hp_handle(timestamp, &target_path, resources);
        self.cleanup(timestamp);
        key
    }

    pub fn observe_hp_guid_update(
        &mut self,
        timestamp: f64,
        guid: [u8; 16],
        current_hp: f64,
        max_hp: Option<f64>,
        evidence: String,
    ) -> String {
        let handle = hex::encode(guid);
        self.observe_hp_update(
            timestamp,
            ObjectHandleKind::AttributeGuid,
            handle,
            current_hp,
            max_hp,
            evidence,
            0.65,
            0.70,
        )
    }

    pub fn observe_net_target_hp_update(
        &mut self,
        timestamp: f64,
        source: &str,
        token: &[u8],
        current_hp: f64,
        max_hp: Option<f64>,
        evidence: String,
    ) -> String {
        let handle = format!("{source}:{}", hex::encode(token));
        self.observe_hp_update(
            timestamp,
            ObjectHandleKind::NetRefHandleCandidate,
            handle,
            current_hp,
            max_hp,
            evidence,
            0.45,
            0.60,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn observe_hp_update(
        &mut self,
        timestamp: f64,
        handle_kind: ObjectHandleKind,
        handle: String,
        current_hp: f64,
        max_hp: Option<f64>,
        evidence: String,
        initial_confidence: f32,
        observed_confidence: f32,
    ) -> String {
        let key = object_key(&handle_kind, &handle);
        let link_handle_kind = match handle_kind {
            ObjectHandleKind::AttributeGuid | ObjectHandleKind::NetRefHandleCandidate => {
                Some(handle_kind.clone())
            }
            _ => None,
        };
        let descriptor = self
            .objects
            .entry(key.clone())
            .or_insert_with(|| ObjectDescriptor {
                handle_kind: handle_kind.clone(),
                handle: handle.clone(),
                class_path: None,
                object_path: None,
                display_name: None,
                table_resolved_name: false,
                owner_handle: None,
                actor_handle: None,
                component_handle: None,
                hp_current: None,
                hp_max: max_hp,
                first_seen_at: timestamp,
                last_seen_at: timestamp,
                evidence: Vec::new(),
                confidence: initial_confidence,
                hp_history: VecDeque::new(),
            });
        if is_new_hp_encounter(descriptor, timestamp, current_hp) {
            descriptor.class_path = None;
            descriptor.object_path = None;
            descriptor.display_name = None;
            descriptor.table_resolved_name = false;
            descriptor.hp_max = max_hp;
            descriptor.first_seen_at = timestamp;
            descriptor.confidence = initial_confidence;
            descriptor.hp_history.clear();
            descriptor.evidence.clear();
            push_unique_evidence(
                &mut descriptor.evidence,
                format!("hp_encounter_reset:{current_hp:.0}"),
            );
        }
        descriptor.last_seen_at = timestamp;
        descriptor.hp_current = Some(current_hp);
        descriptor.hp_max = descriptor.hp_max.or(max_hp);
        descriptor.confidence = descriptor.confidence.max(observed_confidence);
        descriptor.hp_history.push_back(HpObservation {
            timestamp,
            current: current_hp,
            max: max_hp,
            evidence: evidence.clone(),
        });
        while descriptor.hp_history.len() > MAX_HP_HISTORY_PER_OBJECT {
            descriptor.hp_history.pop_front();
        }
        push_unique_evidence(&mut descriptor.evidence, evidence);
        if let Some(link_handle_kind) = link_handle_kind {
            self.link_hp_handle_to_best_path(&key, timestamp, link_handle_kind);
        }
        self.cleanup(timestamp);
        key
    }

    #[allow(dead_code)]
    pub fn observe_possible_handle(
        &mut self,
        timestamp: f64,
        handle_kind: ObjectHandleKind,
        handle: String,
        evidence: String,
    ) -> String {
        let key = object_key(&handle_kind, &handle);
        let descriptor = self
            .objects
            .entry(key.clone())
            .or_insert_with(|| ObjectDescriptor {
                handle_kind,
                handle,
                class_path: None,
                object_path: None,
                display_name: None,
                table_resolved_name: false,
                owner_handle: None,
                actor_handle: None,
                component_handle: None,
                hp_current: None,
                hp_max: None,
                first_seen_at: timestamp,
                last_seen_at: timestamp,
                evidence: Vec::new(),
                confidence: 0.25,
                hp_history: VecDeque::new(),
            });
        descriptor.last_seen_at = timestamp;
        push_unique_evidence(&mut descriptor.evidence, evidence);
        self.cleanup(timestamp);
        key
    }

    #[allow(clippy::too_many_arguments)]
    pub fn observe_path_handle_candidate(
        &mut self,
        timestamp: f64,
        handle_kind: ObjectHandleKind,
        handle: String,
        path: &str,
        resources: &ResourceIndex,
        evidence: String,
        score: u16,
    ) -> String {
        let key = object_key(&handle_kind, &handle);
        let confidence = (score as f32 / 255.0).clamp(0.25, 0.55);
        let has_resolved_name = resources.resolved_name_for_path(path).is_some();
        let display_name = resources.display_name_for_path(path);
        let descriptor = self
            .objects
            .entry(key.clone())
            .or_insert_with(|| ObjectDescriptor {
                handle_kind,
                handle,
                class_path: None,
                object_path: None,
                display_name: None,
                table_resolved_name: false,
                owner_handle: None,
                actor_handle: None,
                component_handle: None,
                hp_current: None,
                hp_max: None,
                first_seen_at: timestamp,
                last_seen_at: timestamp,
                evidence: Vec::new(),
                confidence,
                hp_history: VecDeque::new(),
            });
        descriptor.last_seen_at = timestamp;
        descriptor.confidence = descriptor.confidence.max(confidence);
        match descriptor.object_path.as_deref() {
            Some(existing) if existing != path => push_unique_evidence(
                &mut descriptor.evidence,
                format!("conflicting_path_anchor:{path}"),
            ),
            _ => {
                descriptor.object_path = Some(path.to_owned());
                if path.contains("/Game/") {
                    descriptor.class_path = Some(path.to_owned());
                }
                descriptor.display_name = descriptor.display_name.clone().or(display_name);
                descriptor.table_resolved_name |= has_resolved_name;
            }
        }
        push_unique_evidence(&mut descriptor.evidence, evidence);
        self.cleanup(timestamp);
        key
    }

    pub fn objects_near_time(&self, timestamp: f64, window_seconds: f64) -> Vec<&ObjectDescriptor> {
        self.objects
            .values()
            .filter(|object| (timestamp - object.last_seen_at).abs() <= window_seconds)
            .collect()
    }

    pub fn candidates_for_damage(&self, timestamp: f64) -> Vec<&ObjectDescriptor> {
        self.objects
            .values()
            .filter(|object| object_is_near_damage(object, timestamp))
            .filter(|object| {
                object.hp_current.is_some()
                    || object.table_resolved_name
                    || object
                        .object_path
                        .as_deref()
                        .or(object.class_path.as_deref())
                        .is_some_and(is_targetish_path)
            })
            .collect()
    }

    fn link_path_to_single_hp_handle(
        &mut self,
        timestamp: f64,
        path: &str,
        resources: &ResourceIndex,
    ) {
        if !is_targetish_path(path) {
            return;
        }
        let strong_paths =
            self.strong_targetish_paths_near(timestamp, ATTRIBUTE_PATH_LINK_WINDOW_SECONDS);
        if strong_paths.is_empty() {
            return;
        }
        if strong_paths.len() > 1 {
            for object in self
                .objects
                .values_mut()
                .filter(|object| is_linkable_hp_handle_kind(&object.handle_kind))
                .filter(|object| {
                    (timestamp - object.last_seen_at).abs() <= ATTRIBUTE_PATH_LINK_WINDOW_SECONDS
                })
            {
                mark_ambiguous_path_link(object, strong_paths.len(), &strong_paths);
            }
            return;
        }
        let linkable_keys = self
            .objects
            .iter()
            .filter(|(_, object)| is_linkable_hp_handle_kind(&object.handle_kind))
            .filter(|(_, object)| {
                (timestamp - object.last_seen_at).abs() <= ATTRIBUTE_PATH_LINK_WINDOW_SECONDS
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        if linkable_keys.len() != 1 {
            return;
        }
        let linked_path = strong_paths[0].clone();
        let display_name = resources.display_name_for_path(&linked_path);
        let table_resolved_name = resources.resolved_name_for_path(&linked_path).is_some();
        if let Some(object) = self.objects.get_mut(&linkable_keys[0]) {
            apply_path_link(object, &linked_path, display_name, table_resolved_name);
        }
    }

    fn link_hp_handle_to_best_path(
        &mut self,
        object_key: &str,
        timestamp: f64,
        handle_kind: ObjectHandleKind,
    ) {
        let strong_paths =
            self.strong_targetish_paths_near(timestamp, ATTRIBUTE_PATH_LINK_WINDOW_SECONDS);
        if strong_paths.len() > 1 {
            if let Some(object) = self.objects.get_mut(object_key) {
                mark_ambiguous_path_link(object, strong_paths.len(), &strong_paths);
            }
            return;
        }
        let Some(path) = strong_paths.first() else {
            return;
        };
        let linkable_keys = self
            .objects
            .iter()
            .filter(|(_, object)| object.handle_kind == handle_kind)
            .filter(|(_, object)| {
                (timestamp - object.last_seen_at).abs() <= ATTRIBUTE_PATH_LINK_WINDOW_SECONDS
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        if linkable_keys.len() != 1 || linkable_keys[0] != object_key {
            return;
        }
        let (display_name, table_resolved_name) = self
            .objects
            .values()
            .find(|object| object.object_path.as_deref() == Some(path.as_str()))
            .map(|object| (object.display_name.clone(), object.table_resolved_name))
            .unwrap_or((None, false));
        if let Some(object) = self.objects.get_mut(object_key) {
            apply_path_link(object, path, display_name, table_resolved_name);
        }
    }

    fn strong_targetish_paths_near(&self, timestamp: f64, window_seconds: f64) -> Vec<String> {
        let path_objects = self
            .objects
            .values()
            .filter(|object| object.handle_kind == ObjectHandleKind::PathOnly)
            .filter(|object| (timestamp - object.last_seen_at).abs() <= window_seconds)
            .filter_map(|object| {
                let path = object.object_path.as_deref()?;
                strong_targetish_path(object, path).then_some(object)
            })
            .collect::<Vec<_>>();
        dominant_targetish_paths(path_objects)
    }

    pub fn cleanup(&mut self, timestamp: f64) {
        self.objects
            .retain(|_, object| timestamp - object.last_seen_at <= OBJECT_TTL_SECONDS);
        if self.objects.len() <= MAX_OBJECTS {
            return;
        }
        let mut keys = self
            .objects
            .iter()
            .map(|(key, object)| (key.clone(), object.last_seen_at))
            .collect::<Vec<_>>();
        keys.sort_by(|left, right| left.1.total_cmp(&right.1));
        let remove_count = self.objects.len() - MAX_OBJECTS;
        for (key, _) in keys.into_iter().take(remove_count) {
            self.objects.remove(&key);
        }
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.objects.len()
    }
}

#[derive(Clone, Debug)]
struct TargetPathGroup {
    path: String,
    weight: usize,
    representative_rank: i32,
    last_seen_at: f64,
}

fn dominant_targetish_paths(path_objects: Vec<&ObjectDescriptor>) -> Vec<String> {
    let mut groups = HashMap::<String, TargetPathGroup>::new();
    for object in path_objects {
        let Some(path) = object.object_path.as_deref() else {
            continue;
        };
        let group_key = target_group_key(path).unwrap_or_else(|| path.to_ascii_lowercase());
        let weight = path_observation_weight(object);
        let rank = target_path_representative_rank(path);
        groups
            .entry(group_key)
            .and_modify(|group| {
                group.weight += weight;
                if rank > group.representative_rank
                    || (rank == group.representative_rank
                        && object.last_seen_at > group.last_seen_at)
                {
                    group.path = path.to_owned();
                    group.representative_rank = rank;
                    group.last_seen_at = object.last_seen_at;
                }
            })
            .or_insert_with(|| TargetPathGroup {
                path: path.to_owned(),
                weight,
                representative_rank: rank,
                last_seen_at: object.last_seen_at,
            });
    }

    let mut groups = groups.into_values().collect::<Vec<_>>();
    groups.sort_by(|left, right| {
        right
            .weight
            .cmp(&left.weight)
            .then_with(|| right.representative_rank.cmp(&left.representative_rank))
            .then_with(|| right.last_seen_at.total_cmp(&left.last_seen_at))
            .then_with(|| left.path.cmp(&right.path))
    });

    if groups.is_empty() {
        return Vec::new();
    }
    if groups.len() == 1 {
        return vec![groups[0].path.clone()];
    }
    if groups[0].weight >= 2 && groups[0].weight > groups[1].weight {
        return vec![groups[0].path.clone()];
    }

    let mut paths = groups
        .into_iter()
        .map(|group| group.path)
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

fn path_observation_weight(object: &ObjectDescriptor) -> usize {
    object
        .evidence
        .iter()
        .filter(|entry| entry.starts_with("path_candidate:"))
        .count()
        .max(1)
}

fn target_path_representative_rank(path: &str) -> i32 {
    let lower = path.to_ascii_lowercase();
    let mut rank = 0;
    if lower.starts_with("worldboss_boss") {
        rank += 40;
    }
    if lower.contains("/monster/") {
        rank += 30;
    }
    if lower.contains("_bp") {
        rank += 20;
    }
    if lower.contains("worldboss") {
        rank += 20;
    }
    for weak_marker in ["entrance", "exit", "summon", "spawn", "drop"] {
        if lower.contains(weak_marker) {
            rank -= 15;
        }
    }
    rank
}

fn target_group_key(path: &str) -> Option<String> {
    let lower = path.to_ascii_lowercase();
    for (prefix, marker) in [
        ("boss", "worldboss_boss"),
        ("boss", "worldboss_"),
        ("boss", "boss_"),
        ("boss", "boss"),
        ("mon", "mon_"),
        ("mon", "mon"),
    ] {
        if let Some(number) = number_after_marker(&lower, marker) {
            return Some(format!("{prefix}_{number}"));
        }
    }
    None
}

fn number_after_marker(value: &str, marker: &str) -> Option<u32> {
    let mut search_start = 0;
    while let Some(relative_index) = value[search_start..].find(marker) {
        let digit_start = search_start + relative_index + marker.len();
        let digits = value[digit_start..]
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .collect::<String>();
        if !digits.is_empty() {
            return digits.parse::<u32>().ok();
        }
        search_start = digit_start;
        if search_start >= value.len() {
            break;
        }
    }
    None
}

fn is_new_hp_encounter(object: &ObjectDescriptor, timestamp: f64, current_hp: f64) -> bool {
    let Some(previous_hp) = object.hp_current else {
        return false;
    };
    if timestamp - object.last_seen_at < 2.0 {
        return false;
    }
    current_hp - previous_hp > 500_000.0 && current_hp > previous_hp * 3.0
}

fn apply_path_link(
    attribute: &mut ObjectDescriptor,
    path: &str,
    display_name: Option<String>,
    table_resolved_name: bool,
) {
    if let Some(existing_path) = attribute.object_path.as_deref()
        && existing_path != path
    {
        push_unique_evidence(
            &mut attribute.evidence,
            format!("conflicting_path_link:{path}"),
        );
        return;
    }
    attribute.object_path = Some(path.to_owned());
    attribute.table_resolved_name |= table_resolved_name;
    if path.contains("/Game/") {
        attribute.class_path = Some(path.to_owned());
    }
    attribute.display_name = attribute.display_name.clone().or(display_name);
    attribute.confidence = attribute.confidence.max(0.80);
    push_unique_evidence(&mut attribute.evidence, format!("linked_path:{path}"));
}

fn is_linkable_hp_handle_kind(handle_kind: &ObjectHandleKind) -> bool {
    matches!(
        handle_kind,
        ObjectHandleKind::AttributeGuid | ObjectHandleKind::NetRefHandleCandidate
    )
}

fn path_candidate_confidence(score: u16, target_path: &str, has_resolved_name: bool) -> f32 {
    let base = (score as f32 / 255.0).clamp(0.1, 0.45);
    if has_resolved_name || is_targetish_path(target_path) {
        base.max(0.75)
    } else {
        base
    }
}

fn mark_ambiguous_path_link(attribute: &mut ObjectDescriptor, count: usize, paths: &[String]) {
    if attribute.object_path.is_none() {
        attribute.class_path = None;
        attribute.display_name = None;
        attribute.table_resolved_name = false;
    }
    push_unique_evidence(
        &mut attribute.evidence,
        format!("ambiguous_path_link:{count}"),
    );
    for path in paths.iter().take(3) {
        push_unique_evidence(&mut attribute.evidence, format!("ambiguous_path:{path}"));
    }
}

fn strong_targetish_path(object: &ObjectDescriptor, path: &str) -> bool {
    is_targetish_path(path)
        && (path.starts_with("/Game/") || is_world_boss_path(path) || object.confidence >= 0.70)
}

fn object_key(kind: &ObjectHandleKind, handle: &str) -> String {
    format!("{}:{handle}", kind.label())
}

fn push_unique_evidence(evidence: &mut Vec<String>, value: String) {
    if evidence.iter().any(|item| item == &value) {
        return;
    }
    evidence.push(value);
    if evidence.len() > MAX_EVIDENCE_PER_OBJECT {
        evidence.remove(0);
    }
}

fn damage_candidate_window(object: &ObjectDescriptor) -> f64 {
    if object.hp_current.is_some() {
        return 1.0;
    }
    let target_path = object
        .object_path
        .as_deref()
        .or(object.class_path.as_deref());
    if object.table_resolved_name || target_path.is_some_and(is_precise_target_path) {
        ATTRIBUTE_PATH_LINK_WINDOW_SECONDS
    } else {
        1.0
    }
}

fn object_is_near_damage(object: &ObjectDescriptor, timestamp: f64) -> bool {
    if (timestamp - object.last_seen_at).abs() <= damage_candidate_window(object) {
        return true;
    }
    if object_has_named_hp_target(object)
        && timestamp + HP_HISTORY_DAMAGE_WINDOW_SECONDS >= object.first_seen_at
        && timestamp <= object.last_seen_at + ATTRIBUTE_PATH_LINK_WINDOW_SECONDS
    {
        return true;
    }
    object.hp_history.iter().any(|observation| {
        (timestamp - observation.timestamp).abs() <= HP_HISTORY_DAMAGE_WINDOW_SECONDS
    })
}

fn object_has_named_hp_target(object: &ObjectDescriptor) -> bool {
    object.hp_current.is_some()
        && object.display_name.is_some()
        && object.table_resolved_name
        && object
            .object_path
            .as_deref()
            .or(object.class_path.as_deref())
            .is_some_and(|path| object.table_resolved_name || is_targetish_path(path))
}

pub fn is_targetish_path(value: &str) -> bool {
    if is_ignored_non_target_path(value) {
        return false;
    }
    if is_precise_target_path(value) || is_world_boss_path(value) {
        return true;
    }
    let lower = value.to_ascii_lowercase();
    ["enemy", "npc", "htcharacter"]
        .iter()
        .any(|needle| lower.contains(needle))
}

pub fn is_precise_target_path(value: &str) -> bool {
    if is_ignored_non_target_path(value) || is_world_boss_path(value) {
        return false;
    }
    target_group_key(value).is_some()
}

pub fn is_ignored_non_target_path(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let basename = lower
        .rsplit('/')
        .next()
        .unwrap_or(&lower)
        .rsplit('.')
        .next()
        .unwrap_or(&lower)
        .trim_end_matches("_c")
        .strip_prefix("default__")
        .unwrap_or_else(|| {
            lower
                .rsplit('/')
                .next()
                .unwrap_or(&lower)
                .rsplit('.')
                .next()
                .unwrap_or(&lower)
                .trim_end_matches("_c")
        });
    basename.starts_with("buff_")
        || basename.starts_with("ge_")
        || basename.starts_with("ga_")
        || basename.starts_with("drop")
        || basename.starts_with("dropbox")
        || lower.contains("drop_mon_")
        || lower.contains("/drop/")
        || lower.contains("/dropbox/")
        || basename.contains("lockhp")
        || lower.contains("/monsterbase/")
        || lower.contains("/abilities/")
        || lower.contains("/ability/")
        || lower.contains("/buff/")
        || lower.contains("/effect/")
        || lower.contains("/cooldown/")
        || lower.contains("/passiveeffect/")
}

fn is_world_boss_path(value: &str) -> bool {
    value.to_ascii_lowercase().contains("worldboss")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource_index::ResourceIndex;

    fn path(value: &str) -> PathCandidate {
        path_at(value, 1)
    }

    fn path_at(value: &str, byte_offset: usize) -> PathCandidate {
        PathCandidate {
            value: value.to_owned(),
            byte_offset,
            bit_shift: 0,
            score: 240,
        }
    }

    #[test]
    fn hp_guid_update_maintains_timeline() {
        let mut store = ObjectStateStore::default();
        let guid = [1_u8; 16];
        store.observe_hp_guid_update(1.0, guid, 900.0, None, "hp=900".to_owned());
        let key = store.observe_hp_guid_update(1.5, guid, 800.0, None, "hp=800".to_owned());
        let object = store.objects.get(&key).unwrap();
        assert_eq!(object.hp_history.len(), 2);
        assert_eq!(object.hp_current, Some(800.0));
    }

    #[test]
    fn net_target_hp_update_uses_separate_handle_kind() {
        let mut store = ObjectStateStore::default();
        let token = [9_u8; 40];
        let key = store.observe_net_target_hp_update(
            1.0,
            "currenthp",
            &token,
            900.0,
            None,
            "hp=900".to_owned(),
        );

        let object = store.objects.get(&key).unwrap();
        assert_eq!(object.handle_kind, ObjectHandleKind::NetRefHandleCandidate);
        assert!(object.handle.starts_with("currenthp:"));
        assert_eq!(object.hp_current, Some(900.0));
    }

    #[test]
    fn net_target_hp_update_links_unique_resolved_bare_boss_path() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::load_default();
        store.observe_path_candidate(1.0, &path("Boss_07_BP_DiyBoss"), &resources);

        let key = store.observe_net_target_hp_update(
            1.1,
            "currenthp",
            &[0x10, 0x44, 0x55, 0x66],
            1_976_104.0,
            None,
            "current_hp:10445566=1976104@0:0".to_owned(),
        );

        let object = store.objects.get(&key).unwrap();
        assert_eq!(object.object_path.as_deref(), Some("Boss_07_BP_DiyBoss"));
        assert_eq!(object.display_name.as_deref(), Some("塞润尼缇"));
        assert!(
            object
                .evidence
                .iter()
                .any(|evidence| evidence == "linked_path:Boss_07_BP_DiyBoss")
        );
    }

    #[test]
    fn hp_history_keeps_old_damage_candidate_after_late_path_link() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::load_default();
        let guid = [0x33_u8; 16];
        store.observe_hp_guid_update(10.0, guid, 1000.0, None, "hp=1000".to_owned());
        store.observe_hp_guid_update(10.2, guid, 900.0, None, "hp=900".to_owned());
        store.observe_path_candidate(
            12.8,
            &path("mon_33_BP_World_Perform_C_2147325373"),
            &resources,
        );

        let candidates = store.candidates_for_damage(10.1);

        assert!(candidates.iter().any(|candidate| {
            candidate.handle_kind == ObjectHandleKind::AttributeGuid
                && candidate.object_path.as_deref() == Some("mon_33_BP_World_Perform_C_2147325373")
                && candidate.display_name.as_deref() == Some("流梦种")
        }));
    }

    #[test]
    fn named_hp_target_covers_old_damage_after_history_rollover() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::load_default();
        let guid = [0x44_u8; 16];
        for index in 0..40 {
            store.observe_hp_guid_update(
                10.0 + f64::from(index),
                guid,
                10_000.0 - f64::from(index),
                None,
                format!("hp={}", 10_000 - index),
            );
        }
        store.observe_path_candidate(49.5, &path("Boss_016_BP_DiyBoss_C_2147307033"), &resources);

        let candidates = store.candidates_for_damage(10.1);

        assert!(candidates.iter().any(|candidate| {
            candidate.handle_kind == ObjectHandleKind::AttributeGuid
                && candidate.object_path.as_deref() == Some("Boss_016_BP_DiyBoss_C_2147307033")
                && candidate.display_name.as_deref() == Some("随心泥")
        }));
    }

    #[test]
    fn path_candidate_links_existing_unique_net_target_hp_update() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::load_default();
        let key = store.observe_net_target_hp_update(
            1.0,
            "currenthp",
            &[0x10, 0x44, 0x55, 0x66],
            1_976_104.0,
            None,
            "current_hp:10445566=1976104@0:0".to_owned(),
        );
        store.observe_path_candidate(1.1, &path("Boss_07_BP_DiyBoss"), &resources);

        let object = store.objects.get(&key).unwrap();
        assert_eq!(object.object_path.as_deref(), Some("Boss_07_BP_DiyBoss"));
        assert_eq!(object.display_name.as_deref(), Some("塞润尼缇"));
    }

    #[test]
    fn path_handle_candidate_keeps_anchor_path() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::default();
        let key = store.observe_path_handle_candidate(
            1.0,
            ObjectHandleKind::NetGuidCandidate,
            "0x12345678".to_owned(),
            "/Game/Blueprints/Character/Monster/boss_07/BP_Boss_07.BP_Boss_07_C",
            &resources,
            "net_identity:netguid32=0x12345678 path_anchor:/Game/Blueprints/Character/Monster/boss_07/BP_Boss_07.BP_Boss_07_C@4:0".to_owned(),
            82,
        );

        let object = store.objects.get(&key).unwrap();
        assert_eq!(object.handle_kind, ObjectHandleKind::NetGuidCandidate);
        assert!(
            object
                .object_path
                .as_deref()
                .is_some_and(|path| path.contains("boss_07"))
        );
        assert_eq!(object.display_name.as_deref(), Some("BP_Boss_07"));
    }

    #[test]
    fn path_candidate_is_saved_and_deduped() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::default();
        let candidate = path("/Game/Blueprints/Character/Monster/boss_07/BP.BP_C");
        store.observe_path_candidate(1.0, &candidate, &resources);
        store.observe_path_candidate(1.1, &candidate, &resources);
        assert_eq!(store.len(), 1);
        let object = store.candidates_for_damage(1.1).pop().unwrap();
        assert_eq!(object.evidence.len(), 1);
    }

    #[test]
    fn cleanup_keeps_recent_objects() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::default();
        store.observe_path_candidate(1.0, &path("/Game/Monster/old"), &resources);
        store.observe_path_candidate(25.0, &path("/Game/Monster/recent"), &resources);
        assert_eq!(store.len(), 1);
        assert!(
            store
                .objects
                .values()
                .any(|object| object.handle.contains("recent"))
        );
    }

    #[test]
    fn attribute_guid_links_nearby_unique_target_path() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::default();
        let candidate = path("/Game/Blueprints/Character/Monster/boss_07/BP_Boss_07.BP_Boss_07_C");
        store.observe_path_candidate(1.0, &candidate, &resources);
        let key = store.observe_hp_guid_update(1.1, [2_u8; 16], 900.0, None, "hp=900".to_owned());
        let object = store.objects.get(&key).unwrap();
        assert!(
            object
                .object_path
                .as_deref()
                .is_some_and(|path| path.contains("boss_07"))
        );
        assert_eq!(object.display_name.as_deref(), Some("BP_Boss_07"));
        assert!(
            object
                .evidence
                .iter()
                .any(|evidence| evidence.starts_with("linked_path:"))
        );
    }

    #[test]
    fn monster_folder_buff_path_is_not_targetish() {
        let buff_path = "/Game/Blueprints/Character/Monster/mon_16/Trial/buff_Trial_LockHP100_BP";
        assert!(!is_targetish_path(buff_path));

        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::default();
        store.observe_path_candidate(1.0, &path(buff_path), &resources);
        let key = store.observe_hp_guid_update(1.1, [14_u8; 16], 900.0, None, "hp=900".to_owned());

        let object = store.objects.get(&key).unwrap();
        assert!(object.object_path.is_none());
        assert!(object.display_name.is_none());
    }

    #[test]
    fn monster_base_and_drop_paths_are_not_targetish() {
        assert!(!is_targetish_path(
            "/Game/Blueprints/Level/World/MonsterBase/StorageCabine/Level/StorageCabine_Base"
        ));
        assert!(!is_targetish_path("drop_Mon_OrdinaryMonMaterial_01"));
    }

    #[test]
    fn precise_monster_tag_remains_damage_candidate_for_spawned_target() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::load_default();
        store.observe_path_candidate(30.952, &path("mon_04_BP"), &resources);

        let candidates = store.candidates_for_damage(33.958);

        assert!(candidates.iter().any(|candidate| {
            candidate.object_path.as_deref() == Some("mon_04_BP")
                && candidate.display_name.as_deref() == Some("迷失种")
        }));
    }

    #[test]
    fn attribute_guid_links_nearby_world_boss_id_path() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::default();
        store.observe_path_candidate(9.5, &path("WorldBoss_Boss33"), &resources);
        let key = store.observe_hp_guid_update(10.0, [13_u8; 16], 900.0, None, "hp=900".to_owned());

        let object = store.objects.get(&key).unwrap();
        assert_eq!(object.object_path.as_deref(), Some("WorldBoss_Boss33"));
        assert!(object.confidence >= 0.80);
    }

    #[test]
    fn abyss_stage_tag_links_upper_boss_instead_of_stale_world_boss_path() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::load_default();
        store.observe_path_candidate(20.132, &path("WorldBoss_Boss33"), &resources);
        store.observe_path_candidate(26.556, &path("Abyss_3_11_0"), &resources);

        let key = store.observe_hp_guid_update(
            29.673,
            [
                0x21, 0xf0, 0x4e, 0x92, 0x89, 0x95, 0x33, 0x4f, 0x8c, 0x0b, 0xbc, 0xaa, 0x0e, 0xe1,
                0x6f, 0xe7,
            ],
            1_933_529.0,
            Some(1_940_137.0),
            "boss_hp:21f04e928995334f8c0bbcaa0ee16fe7=1933529".to_owned(),
        );

        let object = store.objects.get(&key).unwrap();
        assert_eq!(object.object_path.as_deref(), Some("Boss_017_BP"));
        assert_eq!(object.display_name.as_deref(), Some("玛门"));
    }

    #[test]
    fn reused_boss_hp_handle_relinks_after_new_encounter_hp_reset() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::load_default();
        let guid = [
            0x21, 0xf0, 0x4e, 0x92, 0x89, 0x95, 0x33, 0x4f, 0x8c, 0x0b, 0xbc, 0xaa, 0x0e, 0xe1,
            0x6f, 0xe7,
        ];
        store.observe_path_candidate(20.132, &path("WorldBoss_Boss33"), &resources);
        store.observe_path_candidate(26.556, &path("Abyss_3_11_0"), &resources);
        let key = store.observe_hp_guid_update(
            29.673,
            guid,
            1_933_529.0,
            None,
            "boss_hp:21f04e928995334f8c0bbcaa0ee16fe7=1933529".to_owned(),
        );
        assert_eq!(
            store.objects.get(&key).unwrap().object_path.as_deref(),
            Some("Boss_017_BP")
        );

        store.observe_hp_guid_update(
            99.194,
            guid,
            1.0,
            None,
            "boss_hp:21f04e928995334f8c0bbcaa0ee16fe7=1".to_owned(),
        );
        store.observe_path_candidate(106.455, &path("Abyss_3_11_1"), &resources);
        store.observe_path_candidate(107.094, &path("Boss_06_BP"), &resources);
        store.observe_hp_guid_update(
            107.462,
            guid,
            1_938_243.0,
            None,
            "boss_hp:21f04e928995334f8c0bbcaa0ee16fe7=1938243".to_owned(),
        );

        let object = store.objects.get(&key).unwrap();
        assert_eq!(object.object_path.as_deref(), Some("Boss_06_BP"));
        assert_eq!(object.display_name.as_deref(), Some("胶卷"));
        assert!(
            object
                .evidence
                .iter()
                .any(|evidence| evidence.starts_with("hp_encounter_reset:"))
        );
    }

    #[test]
    fn dominant_world_boss_group_ignores_single_stale_world_boss_distractor() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::load_default();
        store.observe_path_candidate(5.254, &path_at("WorldBoss_Boss33", 100), &resources);
        store.observe_path_candidate(5.356, &path_at("WorldBoss_Boss33", 200), &resources);
        store.observe_path_candidate(5.362, &path_at("WorldBoss_33_Entrance", 300), &resources);
        store.observe_path_candidate(5.408, &path_at("WorldBoss_33_Entrance", 400), &resources);
        store.observe_path_candidate(7.217, &path_at("WorldBoss_Boss08", 500), &resources);

        let key = store.observe_hp_guid_update(
            8.318,
            [
                0x56, 0x66, 0xdc, 0x34, 0xfb, 0xd7, 0x4d, 0x40, 0xac, 0x8f, 0x67, 0xa9, 0xcc, 0xcf,
                0x18, 0x38,
            ],
            1_712_489.0,
            Some(1_719_495.0),
            "boss_hp:5666dc34fbd74d40ac8f67a9cccf1838=1712489".to_owned(),
        );

        let object = store.objects.get(&key).unwrap();
        assert_eq!(object.object_path.as_deref(), Some("WorldBoss_Boss33"));
        assert_eq!(object.display_name.as_deref(), Some("囿巢鸟"));
        assert!(
            !object
                .evidence
                .iter()
                .any(|evidence| evidence.starts_with("ambiguous_path_link:"))
        );
    }

    #[test]
    fn attribute_guid_does_not_link_ambiguous_target_paths() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::default();
        store.observe_path_candidate(
            1.0,
            &path("/Game/Blueprints/Character/Monster/boss_07/BP_Boss_07.BP_Boss_07_C"),
            &resources,
        );
        store.observe_path_candidate(
            1.05,
            &path("/Game/Blueprints/Character/Monster/boss_08/BP_Boss_08.BP_Boss_08_C"),
            &resources,
        );
        let key = store.observe_hp_guid_update(1.1, [3_u8; 16], 900.0, None, "hp=900".to_owned());
        let object = store.objects.get(&key).unwrap();
        assert!(object.object_path.is_none());
        assert!(object.display_name.is_none());
        assert!(
            object
                .evidence
                .iter()
                .any(|evidence| evidence == "ambiguous_path_link:2")
        );
    }

    #[test]
    fn path_does_not_link_when_multiple_attribute_guids_nearby() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::default();
        let first = store.observe_hp_guid_update(1.0, [4_u8; 16], 900.0, None, "hp=900".to_owned());
        let second =
            store.observe_hp_guid_update(1.05, [5_u8; 16], 800.0, None, "hp=800".to_owned());
        store.observe_path_candidate(
            1.1,
            &path("/Game/Blueprints/Character/Monster/boss_09/BP_Boss_09.BP_Boss_09_C"),
            &resources,
        );
        assert!(store.objects.get(&first).unwrap().object_path.is_none());
        assert!(store.objects.get(&second).unwrap().object_path.is_none());
    }

    #[test]
    fn weak_targetish_path_does_not_replace_unique_strong_path_link() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::default();
        let strong_path = "/Game/Blueprints/Character/Monster/boss_07/BP_Boss_07.BP_Boss_07_C";
        let weak_path = "HTCharacterEnemy";

        store.observe_path_candidate(1.0, &path(strong_path), &resources);
        let key = store.observe_hp_guid_update(1.1, [6_u8; 16], 900.0, None, "hp=900".to_owned());
        store.observe_path_candidate(1.2, &path(weak_path), &resources);

        let object = store.objects.get(&key).unwrap();
        assert_eq!(object.object_path.as_deref(), Some(strong_path));
        assert_eq!(object.display_name.as_deref(), Some("BP_Boss_07"));
        assert!(
            object
                .evidence
                .iter()
                .any(|evidence| evidence == &format!("linked_path:{strong_path}"))
        );
        assert!(
            !object
                .evidence
                .iter()
                .any(|evidence| evidence == &format!("linked_path:{weak_path}"))
        );
    }

    #[test]
    fn later_target_path_does_not_overwrite_existing_attribute_link() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::default();
        let boss_path = "WorldBoss_Boss33";
        let summon_path = "/Game/Blueprints/Character/Monster/mon_41/mon_41_Summon01_BP";

        store.observe_path_candidate(10.0, &path(boss_path), &resources);
        let key = store.observe_hp_guid_update(14.5, [12_u8; 16], 900.0, None, "hp=900".to_owned());
        store.observe_path_candidate(15.0, &path(summon_path), &resources);

        let object = store.objects.get(&key).unwrap();
        assert_eq!(object.object_path.as_deref(), Some(boss_path));
        assert!(
            object
                .evidence
                .iter()
                .any(|evidence| evidence.starts_with("conflicting_path_link:")
                    || evidence.starts_with("ambiguous_path_link:"))
        );
    }

    #[test]
    fn path_first_does_not_link_multiple_attribute_guids_to_same_strong_path() {
        let mut store = ObjectStateStore::default();
        let resources = ResourceIndex::default();
        let strong_path = "/Game/Blueprints/Character/Monster/boss_07/BP_Boss_07.BP_Boss_07_C";

        store.observe_path_candidate(1.0, &path(strong_path), &resources);
        let first = store.observe_hp_guid_update(1.1, [7_u8; 16], 900.0, None, "hp=900".to_owned());
        let second =
            store.observe_hp_guid_update(1.2, [8_u8; 16], 800.0, None, "hp=800".to_owned());

        assert_eq!(
            store.objects.get(&first).unwrap().object_path.as_deref(),
            Some(strong_path)
        );
        assert!(store.objects.get(&second).unwrap().object_path.is_none());
        let linked_count = store
            .objects
            .values()
            .filter(|object| object.handle_kind == ObjectHandleKind::AttributeGuid)
            .filter(|object| object.object_path.as_deref() == Some(strong_path))
            .count();
        assert_eq!(linked_count, 1);
    }
}
