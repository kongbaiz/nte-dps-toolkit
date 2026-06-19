use std::collections::{HashMap, HashSet, VecDeque};

use crate::model::Hit;
use crate::object_state::is_ignored_non_target_path;
use crate::target_resolver::{TargetConfidence, TargetResolutionSummary};

const ACTIVE_LOCK_WINDOW_SECONDS: f64 = 3.0;
const HP_STREAM_CONTINUITY_SECONDS: f64 = 2.5;
const DEAD_HANDLE_REUSE_WINDOW_SECONDS: f64 = 8.0;
const HP_MATCH_TOLERANCE_ABSOLUTE: f64 = 2.0;
const HP_MATCH_TOLERANCE_RATIO: f64 = 0.002;
const MAX_HP_HISTORY_PER_LOCK: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetLockSource {
    RuntimeAlias,
    RuntimeMapping,
    DirectHpTimeline,
    DirectHpDelta,
}

#[derive(Clone, Debug)]
pub struct TargetHpStreamLock {
    pub lock_id: String,
    pub target_id: Option<String>,
    pub target_name: String,
    pub target_path: String,
    pub source: TargetLockSource,
    pub confidence: TargetConfidence,
    pub first_seen_at: f64,
    pub last_seen_at: f64,
    pub last_hp_before: f64,
    pub last_hp_after: f64,
    pub last_hp_reported_max: f64,
    pub generation: u32,
    pub non_hp_alias_keys: HashSet<String>,
    pub hp_alias_keys: HashSet<String>,
    pub dead_at: Option<f64>,
    recent_hp_after: VecDeque<f64>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TargetLockContext {
    pub targetish_path_count: usize,
    pub active_hp_handle_count: usize,
}

#[derive(Clone, Debug, Default)]
pub struct TargetHpStreamLockStore {
    locks: HashMap<String, TargetHpStreamLock>,
    non_hp_alias_index: HashMap<String, String>,
    hp_alias_index: HashMap<String, String>,
    generation_by_path: HashMap<String, u32>,
}

impl TargetHpStreamLockStore {
    pub fn learn_from_named_hit(
        &mut self,
        hit: &mut Hit,
        summary: &mut TargetResolutionSummary,
    ) -> bool {
        if !can_learn_named_hit(hit) {
            return false;
        }
        let Some(target_name) = hit
            .target_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            return false;
        };
        let Some(target_path) = target_context_value(&hit.target_context, "target_path") else {
            return false;
        };
        if is_ignored_non_target_path(target_path) {
            return false;
        }
        let Some(source) = lock_source_from_hit(hit, summary) else {
            return false;
        };
        let target_name = target_name.to_owned();
        let target_path = target_path.to_owned();
        let non_hp_alias_keys = non_hp_alias_keys(hit.target_id.as_ref(), &hit.target_context);
        let hp_alias_keys = hp_alias_keys(hit.target_id.as_ref(), &hit.target_context);

        if self.alias_conflicts_with_existing_lock(
            hit,
            &non_hp_alias_keys,
            &target_path,
            &target_name,
            false,
        ) || self.alias_conflicts_with_existing_lock(
            hit,
            &hp_alias_keys,
            &target_path,
            &target_name,
            true,
        ) {
            summary.target_context = hit.target_context.clone();
            return false;
        }

        let lock_id = self
            .lock_id_for_aliases(&non_hp_alias_keys)
            .or_else(|| self.lock_id_for_same_target(&target_path, &target_name, hit.timestamp))
            .unwrap_or_else(|| self.create_lock_id(&target_path));

        if !self.locks.contains_key(&lock_id) {
            let generation = *self
                .generation_by_path
                .entry(target_path.clone())
                .or_insert(1);
            self.locks.insert(
                lock_id.clone(),
                TargetHpStreamLock {
                    lock_id: lock_id.clone(),
                    target_id: hit.target_id.clone(),
                    target_name: target_name.clone(),
                    target_path: target_path.clone(),
                    source,
                    confidence: summary.confidence,
                    first_seen_at: hit.timestamp,
                    last_seen_at: hit.timestamp,
                    last_hp_before: hit.target_hp_before,
                    last_hp_after: hit.target_hp_after,
                    last_hp_reported_max: hit.target_max_hp,
                    generation,
                    non_hp_alias_keys: HashSet::new(),
                    hp_alias_keys: HashSet::new(),
                    dead_at: None,
                    recent_hp_after: VecDeque::new(),
                },
            );
        }

        let Some(lock) = self.locks.get_mut(&lock_id) else {
            return false;
        };
        if !lock.target_path.eq_ignore_ascii_case(&target_path) {
            push_unique_context(
                &mut hit.target_context,
                "target_conflict=locked_path_mismatch".to_owned(),
            );
            summary.target_context = hit.target_context.clone();
            return false;
        }
        if lock.target_name != target_name {
            push_unique_context(
                &mut hit.target_context,
                "target_conflict=locked_name_mismatch".to_owned(),
            );
            summary.target_context = hit.target_context.clone();
            return false;
        }

        lock.target_id = lock.target_id.clone().or_else(|| hit.target_id.clone());
        lock.source = strongest_source(lock.source, source);
        if summary.confidence.rank() > lock.confidence.rank() {
            lock.confidence = summary.confidence;
        }
        lock.last_seen_at = hit.timestamp;
        lock.last_hp_before = hit.target_hp_before;
        lock.last_hp_after = hit.target_hp_after;
        lock.last_hp_reported_max = hit.target_max_hp;
        lock.dead_at = (hit.target_hp_after <= 1.0).then_some(hit.timestamp);
        push_recent_hp_after(lock, hit.target_hp_after);

        for key in non_hp_alias_keys {
            lock.non_hp_alias_keys.insert(key.clone());
            self.non_hp_alias_index.insert(key, lock_id.clone());
        }
        for key in hp_alias_keys {
            lock.hp_alias_keys.insert(key.clone());
            self.hp_alias_index.insert(key, lock_id.clone());
        }

        replace_or_push_context(&mut hit.target_context, "target_lock_id", &lock.lock_id);
        replace_or_push_context(
            &mut hit.target_context,
            "target_lock_generation",
            &lock.generation.to_string(),
        );
        summary.target_context = hit.target_context.clone();
        true
    }

    pub fn try_apply_to_unnamed_hit(
        &mut self,
        hit: &mut Hit,
        summary: &mut TargetResolutionSummary,
        context: TargetLockContext,
    ) -> bool {
        if !can_apply_to_unnamed_hit(hit) {
            return false;
        }
        let non_hp_aliases = non_hp_alias_keys(hit.target_id.as_ref(), &hit.target_context);
        if let Some(lock_id) = self.unique_lock_for_aliases(&non_hp_aliases, hit.timestamp, false) {
            return self.apply_lock_to_hit(&lock_id, hit, summary, "locked_non_hp_alias");
        }

        let hp_aliases = hp_alias_keys(hit.target_id.as_ref(), &hit.target_context);
        if let Some(lock_id) = self.unique_lock_for_aliases(&hp_aliases, hit.timestamp, true) {
            return self.apply_lock_to_hit(&lock_id, hit, summary, "locked_hp_alias");
        }

        if self.multi_target_guard_blocks(hit.timestamp, context) {
            push_ambiguous_context(hit);
            summary.target_context = hit.target_context.clone();
            return false;
        }

        let mut matches = self
            .active_locks(hit.timestamp)
            .filter(|lock| hp_stream_matches(lock, hit))
            .map(|lock| lock.lock_id.clone())
            .collect::<Vec<_>>();
        matches.sort();
        matches.dedup();
        if matches.len() == 1 {
            return self.apply_lock_to_hit(
                &matches[0],
                hit,
                summary,
                "locked_hp_stream_continuity",
            );
        }
        if matches.len() > 1 {
            push_ambiguous_context(hit);
            summary.target_context = hit.target_context.clone();
        }
        false
    }

    pub fn active_lock_count(&self, timestamp: f64) -> usize {
        self.active_locks(timestamp).count()
    }

    fn create_lock_id(&mut self, target_path: &str) -> String {
        let generation = self
            .generation_by_path
            .entry(target_path.to_owned())
            .and_modify(|value| *value += 1)
            .or_insert(1);
        format!("{target_path}#{generation}")
    }

    fn lock_id_for_aliases(&self, aliases: &HashSet<String>) -> Option<String> {
        aliases
            .iter()
            .find_map(|key| self.non_hp_alias_index.get(key).cloned())
    }

    fn lock_id_for_same_target(
        &self,
        target_path: &str,
        target_name: &str,
        timestamp: f64,
    ) -> Option<String> {
        let mut matches = self
            .active_locks(timestamp)
            .filter(|lock| {
                lock.target_path.eq_ignore_ascii_case(target_path)
                    && lock.target_name == target_name
            })
            .map(|lock| lock.lock_id.clone())
            .collect::<Vec<_>>();
        matches.sort();
        matches.dedup();
        (matches.len() == 1).then(|| matches.remove(0))
    }

    fn unique_lock_for_aliases(
        &self,
        aliases: &HashSet<String>,
        timestamp: f64,
        hp_alias: bool,
    ) -> Option<String> {
        if aliases.is_empty() {
            return None;
        }
        let index = if hp_alias {
            &self.hp_alias_index
        } else {
            &self.non_hp_alias_index
        };
        let mut lock_ids = aliases
            .iter()
            .filter_map(|key| index.get(key))
            .filter_map(|lock_id| self.locks.get(lock_id))
            .filter(|lock| lock_is_active(lock, timestamp))
            .filter(|lock| !hp_alias || !hp_lock_reuse_blocked(lock, timestamp))
            .map(|lock| lock.lock_id.clone())
            .collect::<Vec<_>>();
        lock_ids.sort();
        lock_ids.dedup();
        (lock_ids.len() == 1).then(|| lock_ids.remove(0))
    }

    fn active_locks(&self, timestamp: f64) -> impl Iterator<Item = &TargetHpStreamLock> {
        self.locks
            .values()
            .filter(move |lock| lock_is_active(lock, timestamp))
    }

    fn multi_target_guard_blocks(&self, timestamp: f64, context: TargetLockContext) -> bool {
        self.active_lock_count(timestamp) > 1
            || context.targetish_path_count > 1
            || context.active_hp_handle_count > 1
    }

    fn alias_conflicts_with_existing_lock(
        &self,
        hit: &mut Hit,
        aliases: &HashSet<String>,
        target_path: &str,
        target_name: &str,
        hp_alias: bool,
    ) -> bool {
        let index = if hp_alias {
            &self.hp_alias_index
        } else {
            &self.non_hp_alias_index
        };
        for key in aliases {
            let Some(lock_id) = index.get(key) else {
                continue;
            };
            let Some(lock) = self.locks.get(lock_id) else {
                continue;
            };
            if lock.target_path.eq_ignore_ascii_case(target_path) && lock.target_name == target_name
            {
                continue;
            }
            if hp_alias && hp_lock_reuse_blocked(lock, hit.timestamp) {
                push_unique_context(
                    &mut hit.target_context,
                    "target_conflict=hp_handle_reused_without_lifecycle_reset".to_owned(),
                );
            } else if !lock.target_path.eq_ignore_ascii_case(target_path) {
                push_unique_context(
                    &mut hit.target_context,
                    "target_conflict=locked_path_mismatch".to_owned(),
                );
            } else {
                push_unique_context(
                    &mut hit.target_context,
                    "target_conflict=locked_name_mismatch".to_owned(),
                );
            }
            return true;
        }
        false
    }

    fn apply_lock_to_hit(
        &mut self,
        lock_id: &str,
        hit: &mut Hit,
        summary: &mut TargetResolutionSummary,
        reason: &str,
    ) -> bool {
        let Some(lock) = self.locks.get_mut(lock_id) else {
            return false;
        };
        if hit
            .target_name
            .as_deref()
            .is_some_and(|name| name != lock.target_name)
        {
            push_unique_context(
                &mut hit.target_context,
                "target_conflict=locked_name_mismatch".to_owned(),
            );
            summary.target_context = hit.target_context.clone();
            return false;
        }
        hit.target_id = lock
            .target_id
            .clone()
            .or_else(|| Some(lock.lock_id.clone()));
        hit.target_name = Some(lock.target_name.clone());
        replace_or_push_context(&mut hit.target_context, "target_path", &lock.target_path);
        replace_or_push_context(&mut hit.target_context, "target_name", &lock.target_name);
        replace_or_push_context(
            &mut hit.target_context,
            "target_name_resolution",
            "hp_stream_lock_inherited",
        );
        replace_or_push_context(&mut hit.target_context, "target_lock_id", &lock.lock_id);
        replace_or_push_context(
            &mut hit.target_context,
            "target_lock_generation",
            &lock.generation.to_string(),
        );
        let age = (hit.timestamp - lock.first_seen_at).max(0.0);
        replace_or_push_context(
            &mut hit.target_context,
            "target_lock_age_ms",
            &format!("{:.0}", age * 1000.0),
        );
        push_unique_context(&mut hit.target_context, format!("reason={reason}"));
        lock.last_seen_at = hit.timestamp;
        lock.last_hp_before = hit.target_hp_before;
        lock.last_hp_after = hit.target_hp_after;
        lock.last_hp_reported_max = hit.target_max_hp;
        push_recent_hp_after(lock, hit.target_hp_after);

        summary.target_id = hit.target_id.clone();
        summary.target_name = hit.target_name.clone();
        summary.target_context = hit.target_context.clone();
        summary.score = summary.score.max(80);
        if summary.confidence.rank() < TargetConfidence::Probable.rank() {
            summary.confidence = TargetConfidence::Probable;
        }
        true
    }
}

fn can_learn_named_hit(hit: &Hit) -> bool {
    hit.direction != "incoming"
        && hit
            .target_name
            .as_deref()
            .is_some_and(|name| !name.trim().is_empty())
        && !context_has_any(
            &hit.target_context,
            &[
                "target_unresolved=ambiguous_multi_target",
                "target_suppressed=ambiguous_multi_target",
                "target_name_resolution=state_backfill",
                "reason=last_hp_close_to_hit_after",
                "reason=target_max_hp_only_weak",
                "reason=runtime_unique_active_named_instance",
                "reason=path_only_target_name_suppressed",
                "reason=hp_handle_path_without_direct_hp_suppressed",
                "reason=net_identity_path_anchor_unconfirmed",
            ],
        )
        && !hit
            .target_context
            .iter()
            .any(|entry| entry.starts_with("target_conflict="))
}

fn can_apply_to_unnamed_hit(hit: &Hit) -> bool {
    hit.direction != "incoming"
        && hit.target_name.is_none()
        && hit.target_hp_after > 1.0
        && !context_has_any(
            &hit.target_context,
            &["reason=recent_death_suppressed_stale_target"],
        )
        && !hit
            .target_context
            .iter()
            .any(|entry| entry.starts_with("target_conflict="))
        && !hit
            .target_context
            .iter()
            .any(|entry| entry == "target_unresolved=ambiguous_multi_target")
}

fn lock_source_from_hit(hit: &Hit, summary: &TargetResolutionSummary) -> Option<TargetLockSource> {
    if hit
        .target_context
        .iter()
        .any(|entry| entry.starts_with("reason=runtime_alias:"))
    {
        return Some(TargetLockSource::RuntimeAlias);
    }
    if hit
        .target_context
        .iter()
        .any(|entry| entry == "target_name_resolution=runtime_mapping")
    {
        return Some(TargetLockSource::RuntimeMapping);
    }
    if hit
        .target_context
        .iter()
        .any(|entry| entry == "reason=runtime_hp_timeline_unique")
    {
        return Some(TargetLockSource::DirectHpTimeline);
    }
    if hit.target_context.iter().any(|entry| {
        entry.starts_with("reason=hp_guid_timeline_match")
            || entry.starts_with("reason=net_target_hp_timeline_match")
            || entry.starts_with("reason=hp_timeline_match")
    }) {
        return Some(TargetLockSource::DirectHpTimeline);
    }
    if hit.target_context.iter().any(|entry| {
        entry.starts_with("reason=boss_hp_delta_match")
            || entry.starts_with("reason=net_target_hp_delta_match")
            || entry.starts_with("reason=hp_delta_match")
    }) {
        return Some(TargetLockSource::DirectHpDelta);
    }
    if summary.direct_hp_evidence
        && target_context_value(&hit.target_context, "target_name_resolution")
            == Some("table_resolved")
    {
        return Some(TargetLockSource::DirectHpTimeline);
    }
    None
}

fn hp_stream_matches(lock: &TargetHpStreamLock, hit: &Hit) -> bool {
    let age = hit.timestamp - lock.last_seen_at;
    if !(0.0..=HP_STREAM_CONTINUITY_SECONDS).contains(&age) {
        return false;
    }
    let tolerance = hp_tolerance(lock.last_hp_before.max(hit.target_hp_before));
    if hit.target_hp_after > lock.last_hp_before + tolerance {
        return false;
    }
    let max_denominator = lock.last_hp_reported_max.max(hit.target_max_hp).max(1.0);
    let max_ratio = (lock.last_hp_reported_max - hit.target_max_hp).abs() / max_denominator;
    max_ratio <= 0.03
        || lock
            .recent_hp_after
            .iter()
            .any(|after| hit.target_hp_after <= *after + tolerance)
}

fn lock_is_active(lock: &TargetHpStreamLock, timestamp: f64) -> bool {
    timestamp >= lock.last_seen_at
        && timestamp - lock.last_seen_at <= ACTIVE_LOCK_WINDOW_SECONDS
        && lock
            .dead_at
            .is_none_or(|dead_at| timestamp - dead_at > DEAD_HANDLE_REUSE_WINDOW_SECONDS)
}

fn hp_lock_reuse_blocked(lock: &TargetHpStreamLock, timestamp: f64) -> bool {
    lock.dead_at.is_some_and(|dead_at| {
        timestamp >= dead_at && timestamp - dead_at <= DEAD_HANDLE_REUSE_WINDOW_SECONDS
    })
}

fn strongest_source(left: TargetLockSource, right: TargetLockSource) -> TargetLockSource {
    if lock_source_rank(right) > lock_source_rank(left) {
        right
    } else {
        left
    }
}

fn lock_source_rank(source: TargetLockSource) -> u8 {
    match source {
        TargetLockSource::RuntimeAlias => 4,
        TargetLockSource::RuntimeMapping => 3,
        TargetLockSource::DirectHpTimeline => 2,
        TargetLockSource::DirectHpDelta => 1,
    }
}

fn push_recent_hp_after(lock: &mut TargetHpStreamLock, value: f64) {
    lock.recent_hp_after.push_back(value);
    while lock.recent_hp_after.len() > MAX_HP_HISTORY_PER_LOCK {
        lock.recent_hp_after.pop_front();
    }
}

fn context_has_any(context: &[String], values: &[&str]) -> bool {
    values
        .iter()
        .any(|value| context.iter().any(|entry| entry == value))
}

fn target_context_value<'a>(context: &'a [String], key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    context
        .iter()
        .find_map(|value| value.strip_prefix(&prefix))
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "None")
}

fn target_context_values<'a>(context: &'a [String], key: &str) -> impl Iterator<Item = &'a str> {
    let prefix = format!("{key}=");
    context
        .iter()
        .filter_map(move |value| value.strip_prefix(&prefix))
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "None")
}

fn non_hp_alias_keys(target_id: Option<&String>, context: &[String]) -> HashSet<String> {
    let mut keys = HashSet::new();
    if let Some(target_id) = target_id {
        extend_alias_keys(&mut keys, target_id);
    }
    for key in [
        "actor_channel",
        "iris_ref32",
        "netguid32",
        "netguid_packed",
        "sdk_net_target",
    ] {
        for value in target_context_values(context, key) {
            extend_alias_keys(&mut keys, &format!("{key}:{value}"));
        }
    }
    for value in target_context_values(context, "target_handle_candidate") {
        if !is_hp_alias_key(value) {
            extend_alias_keys(&mut keys, value);
        }
    }
    keys.retain(|key| !is_hp_alias_key(key));
    keys
}

fn hp_alias_keys(target_id: Option<&String>, context: &[String]) -> HashSet<String> {
    let mut keys = HashSet::new();
    if let Some(target_id) = target_id.filter(|target_id| is_hp_alias_key(target_id)) {
        extend_alias_keys(&mut keys, target_id);
    }
    for value in target_context_values(context, "target_handle_candidate") {
        if is_hp_alias_key(value) {
            extend_alias_keys(&mut keys, value);
        }
    }
    for value in target_context_values(context, "boss_hp_guid") {
        extend_alias_keys(&mut keys, &format!("boss_hp_guid:{value}"));
    }
    for value in target_context_values(context, "current_hp_token") {
        extend_alias_keys(&mut keys, &format!("current_hp_token:{value}"));
    }
    keys.retain(|key| is_hp_alias_key(key));
    keys
}

fn extend_alias_keys(keys: &mut HashSet<String>, key: &str) {
    for key in equivalent_alias_keys(key) {
        keys.insert(key);
    }
}

fn equivalent_alias_keys(key: &str) -> Vec<String> {
    let key = normalize_alias_key(key);
    let mut keys = vec![key.clone()];
    if let Some(value) = key.strip_prefix("AttributeGuid:") {
        keys.push(format!("boss_hp_guid:{value}"));
    } else if let Some(value) = key.strip_prefix("boss_hp_guid:") {
        keys.push(format!("AttributeGuid:{value}"));
    } else if let Some(value) = key.strip_prefix("NetRefHandleCandidate:currenthp:") {
        keys.push(format!("current_hp_token:{value}"));
    } else if let Some(value) = key.strip_prefix("current_hp_token:") {
        keys.push(format!("NetRefHandleCandidate:currenthp:{value}"));
    } else if let Some(value) = key.strip_prefix("NetRefHandleCandidate:sdk_target:") {
        keys.push(format!("sdk_net_target:{value}"));
    } else if let Some(value) = key.strip_prefix("sdk_net_target:") {
        keys.push(format!("NetRefHandleCandidate:sdk_target:{value}"));
    } else if let Some(value) = key.strip_prefix("NetRefHandleCandidate:") {
        keys.push(format!("iris_ref32:{value}"));
    } else if let Some(value) = key.strip_prefix("iris_ref32:") {
        keys.push(format!("NetRefHandleCandidate:{value}"));
    } else if let Some(value) = key.strip_prefix("NetGuidCandidate:") {
        keys.push(format!("netguid32:{value}"));
        keys.push(format!("netguid_packed:{value}"));
    } else if let Some(value) = key.strip_prefix("netguid32:") {
        keys.push(format!("NetGuidCandidate:{value}"));
        keys.push(format!("netguid_packed:{value}"));
    } else if let Some(value) = key.strip_prefix("netguid_packed:") {
        keys.push(format!("NetGuidCandidate:{value}"));
        keys.push(format!("netguid32:{value}"));
    }
    keys
}

fn is_hp_alias_key(key: &str) -> bool {
    key.starts_with("AttributeGuid:")
        || key.starts_with("boss_hp_guid:")
        || key.starts_with("current_hp_token:")
        || key.starts_with("NetRefHandleCandidate:currenthp:")
}

fn normalize_alias_key(key: &str) -> String {
    let key = key.trim().split('|').next().unwrap_or(key.trim());
    let Some((kind, value)) = key.split_once(':') else {
        return key.to_owned();
    };
    format!("{kind}:{}", value.trim().to_ascii_lowercase())
}

fn replace_or_push_context(context: &mut Vec<String>, key: &str, value: &str) {
    let prefix = format!("{key}=");
    context.retain(|entry| !entry.starts_with(&prefix));
    push_unique_context(context, format!("{key}={value}"));
}

fn push_unique_context(context: &mut Vec<String>, value: String) -> bool {
    if context.iter().any(|entry| entry == &value) {
        return false;
    }
    context.push(value);
    true
}

fn push_ambiguous_context(hit: &mut Hit) {
    push_unique_context(
        &mut hit.target_context,
        "target_unresolved=ambiguous_multi_target".to_owned(),
    );
    push_unique_context(
        &mut hit.target_context,
        "target_suppressed=ambiguous_multi_target".to_owned(),
    );
}

fn hp_tolerance(scale: f64) -> f64 {
    HP_MATCH_TOLERANCE_ABSOLUTE.max(scale.abs() * HP_MATCH_TOLERANCE_RATIO)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(timestamp: f64, before: f64, after: f64, max: f64) -> Hit {
        Hit {
            timestamp,
            char_id: 1,
            char_name: "tester".to_owned(),
            char_known: true,
            damage: (before - after).abs(),
            byte_offset: 1,
            bit_shift: 0,
            char_source: "test".to_owned(),
            direction: "outgoing".to_owned(),
            target_hp_before: before,
            target_hp_after: after,
            target_max_hp: max,
            target_hp_percent: after / max.max(1.0) * 100.0,
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

    fn named_hit(timestamp: f64, before: f64, after: f64) -> (Hit, TargetResolutionSummary) {
        let mut hit = hit(timestamp, before, after, 1_685_262.0);
        hit.target_name = Some("墨菲克斯".to_owned());
        hit.target_id = Some("boss#1".to_owned());
        hit.target_context = vec![
            "target_path=/Game/Monster/Boss_Morphix.Boss_Morphix_C".to_owned(),
            "target_name=墨菲克斯".to_owned(),
            "target_name_resolution=table_resolved".to_owned(),
            "reason=hp_guid_timeline_match:test".to_owned(),
            "boss_hp_guid=abc".to_owned(),
        ];
        let summary = TargetResolutionSummary {
            target_id: hit.target_id.clone(),
            target_name: hit.target_name.clone(),
            target_context: hit.target_context.clone(),
            score: 120,
            confidence: TargetConfidence::Confirmed,
            direct_hp_evidence: true,
        };
        (hit, summary)
    }

    #[test]
    fn single_boss_hp_stream_inherits_continuous_unnamed_hits() {
        let mut store = TargetHpStreamLockStore::default();
        let (mut hit1, mut summary1) = named_hit(1.0, 1_374_729.0, 1_368_969.0);
        assert!(store.learn_from_named_hit(&mut hit1, &mut summary1));

        let mut hit2 = hit(1.3, 1_368_969.0, 1_369_056.0, 1_685_262.0);
        let mut summary2 = TargetResolutionSummary::default();
        assert!(store.try_apply_to_unnamed_hit(
            &mut hit2,
            &mut summary2,
            TargetLockContext::default()
        ));

        assert_eq!(hit2.target_name.as_deref(), Some("墨菲克斯"));
        assert!(
            hit2.target_context
                .iter()
                .any(|entry| entry == "target_name_resolution=hp_stream_lock_inherited")
        );
        assert!(
            !hit2
                .target_context
                .iter()
                .any(|entry| entry == "target_unresolved=ambiguous_multi_target")
        );
    }

    #[test]
    fn multiple_active_locks_block_stream_inheritance() {
        let mut store = TargetHpStreamLockStore::default();
        let (mut hit1, mut summary1) = named_hit(1.0, 1000.0, 900.0);
        assert!(store.learn_from_named_hit(&mut hit1, &mut summary1));
        let mut hit_b = hit(1.1, 2000.0, 1900.0, 2000.0);
        hit_b.target_name = Some("另一只怪".to_owned());
        hit_b.target_context = vec![
            "target_path=/Game/Monster/Boss_Other.Boss_Other_C".to_owned(),
            "target_name=另一只怪".to_owned(),
            "target_name_resolution=table_resolved".to_owned(),
            "reason=hp_guid_timeline_match:test".to_owned(),
            "boss_hp_guid=def".to_owned(),
        ];
        let mut summary_b = TargetResolutionSummary {
            target_name: hit_b.target_name.clone(),
            target_context: hit_b.target_context.clone(),
            confidence: TargetConfidence::Confirmed,
            score: 120,
            direct_hp_evidence: true,
            ..Default::default()
        };
        assert!(store.learn_from_named_hit(&mut hit_b, &mut summary_b));

        let mut unknown = hit(1.2, 900.0, 850.0, 1000.0);
        let mut summary = TargetResolutionSummary::default();
        assert!(!store.try_apply_to_unnamed_hit(
            &mut unknown,
            &mut summary,
            TargetLockContext::default()
        ));
        assert_eq!(unknown.target_name, None);
        assert!(
            unknown
                .target_context
                .iter()
                .any(|entry| entry == "target_unresolved=ambiguous_multi_target")
        );
    }

    #[test]
    fn hp_alias_unique_match_inherits_inside_scoped_lock() {
        let mut store = TargetHpStreamLockStore::default();
        let (mut hit1, mut summary1) = named_hit(1.0, 1000.0, 900.0);
        assert!(store.learn_from_named_hit(&mut hit1, &mut summary1));

        let mut unknown = hit(1.5, 900.0, 800.0, 1000.0);
        unknown.target_context.push("boss_hp_guid=abc".to_owned());
        let mut summary = TargetResolutionSummary::default();

        assert!(store.try_apply_to_unnamed_hit(
            &mut unknown,
            &mut summary,
            TargetLockContext {
                active_hp_handle_count: 1,
                targetish_path_count: 0,
            },
        ));
        assert_eq!(unknown.target_name.as_deref(), Some("墨菲克斯"));
        assert!(
            unknown
                .target_context
                .iter()
                .any(|entry| entry.starts_with("target_lock_id="))
        );
    }

    #[test]
    fn hp_alias_reuse_conflict_does_not_retarget_lock() {
        let mut store = TargetHpStreamLockStore::default();
        let (mut hit1, mut summary1) = named_hit(1.0, 1000.0, 0.0);
        assert!(store.learn_from_named_hit(&mut hit1, &mut summary1));

        let mut hit_b = hit(2.0, 2000.0, 1900.0, 2000.0);
        hit_b.target_name = Some("另一只怪".to_owned());
        hit_b.target_context = vec![
            "target_path=/Game/Monster/Boss_Other.Boss_Other_C".to_owned(),
            "target_name=另一只怪".to_owned(),
            "target_name_resolution=table_resolved".to_owned(),
            "reason=hp_guid_timeline_match:test".to_owned(),
            "boss_hp_guid=abc".to_owned(),
        ];
        let mut summary_b = TargetResolutionSummary {
            target_name: hit_b.target_name.clone(),
            target_context: hit_b.target_context.clone(),
            confidence: TargetConfidence::Confirmed,
            score: 120,
            direct_hp_evidence: true,
            ..Default::default()
        };

        assert!(!store.learn_from_named_hit(&mut hit_b, &mut summary_b));
        assert_eq!(hit_b.target_name.as_deref(), Some("另一只怪"));
        assert!(
            hit_b.target_context.iter().any(|entry| {
                entry == "target_conflict=hp_handle_reused_without_lifecycle_reset"
            })
        );
    }

    #[test]
    fn screenshot_like_single_lock_sequence_inherits_unknown_rows() {
        let mut store = TargetHpStreamLockStore::default();
        let (mut hit1, mut summary1) = named_hit(1.0, 1_374_729.0, 1_368_969.0);
        assert!(store.learn_from_named_hit(&mut hit1, &mut summary1));

        for (timestamp, before, after, max) in [
            (1.1, 1_368_969.0, 1_369_056.0, 1_685_262.0),
            (1.7, 1_346_337.0, 1_341_619.0, 1_676_678.0),
            (2.3, 1_331_566.0, 1_323_571.0, 1_667_752.0),
        ] {
            let mut unknown = hit(timestamp, before, after, max);
            let mut summary = TargetResolutionSummary::default();
            assert!(store.try_apply_to_unnamed_hit(
                &mut unknown,
                &mut summary,
                TargetLockContext::default()
            ));
            assert_eq!(unknown.target_name.as_deref(), Some("墨菲克斯"));
            assert!(
                unknown
                    .target_context
                    .iter()
                    .any(|entry| { entry == "target_name_resolution=hp_stream_lock_inherited" })
            );
        }
    }

    #[test]
    fn weak_hp_close_context_does_not_learn_lock() {
        let mut store = TargetHpStreamLockStore::default();
        let mut hit = hit(1.0, 1000.0, 900.0, 1000.0);
        hit.target_name = Some("墨菲克斯".to_owned());
        hit.target_context = vec![
            "target_path=/Game/Monster/Boss_Morphix.Boss_Morphix_C".to_owned(),
            "target_name=墨菲克斯".to_owned(),
            "reason=last_hp_close_to_hit_after".to_owned(),
        ];
        let mut summary = TargetResolutionSummary {
            target_name: hit.target_name.clone(),
            target_context: hit.target_context.clone(),
            confidence: TargetConfidence::Probable,
            score: 80,
            direct_hp_evidence: false,
            ..Default::default()
        };

        assert!(!store.learn_from_named_hit(&mut hit, &mut summary));
        assert_eq!(store.active_lock_count(1.0), 0);
    }
}
