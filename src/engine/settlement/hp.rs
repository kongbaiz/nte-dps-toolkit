//! Derived max-HP scaling, validated against consecutive server snapshots for
//! the same full target identity. Never use a client prediction or an arbitrary
//! HP residual as damage. Frozen predecessors also support late requests.
use super::*;

#[derive(Clone, Debug)]
pub(super) struct HpWitness {
    key: MessageKey,
    ordinal: usize,
    hp_bits: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HpAdjustmentKind {
    RuleVerifiedScaling,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HpAdjustment {
    pub kind: HpAdjustmentKind,
    /// Raw server snapshot provenance in the enclosing connection/generation.
    pub preceding_message: String,
    pub preceding_timestamp_bits: String,
    pub preceding_target_ordinal: usize,
    pub hp_before_bits: u32,
    pub max_hp_before_bits: u32,
    pub max_hp_after_bits: u32,
    pub direct_damage: i32,
    pub rule_percent: u32,
}

impl HpAdjustment {
    pub fn reduction(&self) -> f64 {
        f64::from(f32::from_bits(self.max_hp_before_bits))
            - f64::from(f32::from_bits(self.max_hp_after_bits))
    }
    pub fn additional_loss(&self, current_hp_bits: u32) -> f64 {
        f64::from(f32::from_bits(self.hp_before_bits))
            - f64::from(self.direct_damage)
            - f64::from(f32::from_bits(current_hp_bits))
    }
    pub fn valid(&self, current_hp_bits: u32) -> bool {
        let before = f32::from_bits(self.hp_before_bits);
        let maximum = f32::from_bits(self.max_hp_before_bits);
        let next = f32::from_bits(self.max_hp_after_bits);
        let after = f32::from_bits(current_hp_bits);
        if ![before, maximum, next, after].iter().all(|x| x.is_finite())
            || before <= 0.0
            || before > maximum
            || maximum <= 0.0
            || next <= 0.0
            || next >= maximum
            || after < 0.0
            || self.direct_damage <= 0
            || self.rule_percent == 0
            || self.rule_percent > 10_000
            || self.preceding_message.len() > 20
            || self.preceding_timestamp_bits.len() > 20
            || self.preceding_message.parse::<i64>().is_err()
            || self.preceding_timestamp_bits.parse::<u64>().is_err()
            || self.preceding_target_ordinal >= 1024
        {
            return false;
        }
        let expected_max = maximum - self.direct_damage as f32 * (self.rule_percent as f32 / 100.0);
        let ordinary = before - self.direct_damage as f32;
        // Separate f32 operations match the verified divide-then-multiply
        // scaling. No epsilon, closest-match selection or approximate amount.
        ordinary > 0.0
            && expected_max.to_bits() == next.to_bits()
            && ((ordinary / maximum) * next).to_bits() == current_hp_bits
            && self.additional_loss(current_hp_bits) > 0.0
    }
}

impl Ledger {
    pub(super) fn freeze_hp_predecessors(&mut self, settlement: &Settlement) -> Result<(), Error> {
        let key = settlement.key;
        let mut before = Vec::with_capacity(settlement.targets.len());
        let mut staged = HashMap::new();
        let mut new_parents = std::collections::HashSet::new();
        for (ordinal, target) in settlement.targets.iter().enumerate() {
            let prior = staged
                .get(&target.target)
                .or_else(|| self.hp_cursors.get(&target.target))
                .cloned();
            if let Some(witness) = &prior
                && witness.key != key
                && self
                    .hp_dependents
                    .get(&witness.key)
                    .is_none_or(|d| !d.contains(&key))
            {
                new_parents.insert(witness.key);
            }
            before.push(prior);
            staged.insert(
                target.target.clone(),
                HpWitness {
                    key,
                    ordinal,
                    hp_bits: target.current_hp_bits,
                },
            );
        }
        if self.hp_cursors.len()
            + staged
                .keys()
                .filter(|k| !self.hp_cursors.contains_key(*k))
                .count()
            > 4096
            || self.hp_dependency_count + new_parents.len() > self.capacity
        {
            return Err(Error::BudgetExceeded);
        }
        for parent in new_parents {
            self.hp_dependents.entry(parent).or_default().insert(key);
            self.hp_dependency_count += 1;
        }
        self.hp_cursors.extend(staged);
        // Healing breaks the damage-only witness chain. Keep the recovery in
        // the dedup identity, but do not turn its HP value/delta into damage.
        for recovery in &settlement.recoveries {
            self.hp_cursors.remove(&recovery.target);
        }
        self.entry(key)?.hp_before = before;
        Ok(())
    }

    pub(super) fn hp_adjustment(
        &self,
        key: MessageKey,
        ordinal: usize,
        target: &SettledTarget,
        requests: &[&RequestTarget],
        mechanic: Option<&EffectMechanic>,
    ) -> Option<HpAdjustment> {
        let mechanic = mechanic?;
        let entry = self.entries.get(&key)?;
        entry.settlement.as_ref()?;
        if entry.request_conflict
            || entry.settlement_conflict
            || target.components.len() != 1
            || mechanic.max_hp_reduction_percent == 0
            || mechanic.owner.is_none()
            || requests.is_empty()
            || requests
                .iter()
                .any(|r| mechanic.owner != r.source.character_id())
        {
            return None;
        }
        let witness = entry.hp_before.get(ordinal)?.as_ref()?;
        let previous = self.entries.get(&witness.key)?;
        if previous.settlement_conflict {
            return None;
        }
        let previous_target = previous.settlement.as_ref()?.targets.get(witness.ordinal)?;
        if previous_target.target != target.target
            || previous_target.current_hp_bits != witness.hp_bits
        {
            return None;
        }
        let maximum = consensus(requests, |r| r.max_hp_bits)?;
        let damage = target.components[0].damage;
        let after_max = f32::from_bits(maximum)
            - damage as f32 * (mechanic.max_hp_reduction_percent as f32 / 100.0);
        let proof = HpAdjustment {
            kind: HpAdjustmentKind::RuleVerifiedScaling,
            preceding_message: witness.key.message.to_string(),
            preceding_timestamp_bits: witness.key.timestamp_bits.to_string(),
            preceding_target_ordinal: witness.ordinal,
            hp_before_bits: witness.hp_bits,
            max_hp_before_bits: maximum,
            max_hp_after_bits: after_max.to_bits(),
            direct_damage: damage,
            rule_percent: mechanic.max_hp_reduction_percent,
        };
        proof.valid(target.current_hp_bits).then_some(proof)
    }

    /// Conflicting predecessor settlements retract dependent derived values,
    /// not the dependent message's independently observed ordinary damage.
    pub fn invalidate_hp_continuity(&mut self) {
        self.hp_cursors.clear();
    }

    pub fn take_hp_updates(&mut self) -> Vec<Change> {
        let keys = std::mem::take(&mut self.hp_updates);
        keys.into_iter()
            .filter_map(|key| self.project(key))
            .collect()
    }
}
