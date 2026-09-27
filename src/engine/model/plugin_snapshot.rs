//! Immutable per-hit native observations, separate from current live actor state.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotValidationError {
    TooLarge,
    InvalidFormat,
}
impl std::fmt::Display for SnapshotValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
pub const MAX_SNAPSHOT_BYTES: usize = 128 * 1024;
pub const MAX_SESSION_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_SESSION_SNAPSHOTS: usize = 16_384;

fn optional_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Id {
        Number(u64),
        Text(String),
    }
    Option::<Id>::deserialize(d)?
        .map(|value| {
            let text = match value {
                Id::Number(n) => n.to_string(),
                Id::Text(s) => s,
            };
            if text.is_empty()
                || text.len() > 20
                || !text.bytes().all(|c| c.is_ascii_digit())
                || text.parse::<u64>().is_err()
            {
                return Err(serde::de::Error::custom("invalid snapshot identity"));
            }
            Ok(text)
        })
        .transpose()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AttributeValue {
    Number(f64),
    Boolean(bool),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttributeDatum {
    pub value: Option<AttributeValue>,
    pub status: String,
    pub source: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttributeSnapshot {
    pub actor_index: i32,
    pub actor_serial: i32,
    #[serde(default, deserialize_with = "optional_id")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "optional_id")]
    pub generation: Option<String>,
    #[serde(default, deserialize_with = "optional_id")]
    pub sampled_unix_us: Option<String>,
    #[serde(default, deserialize_with = "optional_id")]
    pub finished_unix_us: Option<String>,
    #[serde(default)]
    pub actor_name: Option<String>,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub values: Option<BTreeMap<String, AttributeDatum>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Effect {
    pub instance_key: String,
    pub key: String,
    pub name: String,
    pub description: String,
    pub source: String,
    pub kind: u8,
    pub stacks: i32,
    pub duration: f64,
    pub start_world_time: f64,
    pub level: f64,
    pub inhibited: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectSnapshot {
    pub actor_index: i32,
    pub actor_serial: i32,
    #[serde(default, deserialize_with = "optional_id")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "optional_id")]
    pub observed_us: Option<String>,
    #[serde(default)]
    pub complete: Option<bool>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub effects: Option<Vec<Effect>>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EffectCounts {
    pub positive: usize,
    pub negative: usize,
    pub other: usize,
    pub complete: Option<bool>,
}
impl EffectSnapshot {
    pub fn counts(&self) -> Option<EffectCounts> {
        let rows = self.effects.as_ref()?;
        let positive = rows.iter().filter(|r| r.kind == 2).count();
        let negative = rows.iter().filter(|r| r.kind == 3).count();
        Some(EffectCounts {
            positive,
            negative,
            other: rows.len() - positive - negative,
            complete: self.complete,
        })
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HitSnapshotData {
    pub attacker_attributes: Option<AttributeSnapshot>,
    pub victim_attributes: Option<AttributeSnapshot>,
    pub attacker_effects: Option<EffectSnapshot>,
    pub victim_effects: Option<EffectSnapshot>,
}
impl HitSnapshotData {
    /// Bounded accounting on the native input worker, never in the hot reducer.
    pub fn validate(&self) -> Result<usize, SnapshotValidationError> {
        let mut bytes = std::mem::size_of::<Self>();
        let mut text = |s: &str, limit: usize| {
            if s.len() > limit {
                return Err(SnapshotValidationError::TooLarge);
            }
            bytes += s.len();
            Ok(())
        };
        for attributes in [&self.attacker_attributes, &self.victim_attributes]
            .into_iter()
            .flatten()
        {
            for s in [
                &attributes.actor_name,
                &attributes.stage,
                &attributes.status,
            ]
            .into_iter()
            .flatten()
            {
                text(s, 512)?;
            }
            if let Some(values) = &attributes.values {
                if values.len() > 64 {
                    return Err(SnapshotValidationError::TooLarge);
                }
                for (key, value) in values {
                    text(key, 64)?;
                    text(&value.status, 128)?;
                    text(&value.source, 256)?;
                    if matches!(value.value,Some(AttributeValue::Number(n)) if !n.is_finite()) {
                        return Err(SnapshotValidationError::InvalidFormat);
                    }
                }
            }
        }
        for effects in [&self.attacker_effects, &self.victim_effects]
            .into_iter()
            .flatten()
        {
            if let Some(status) = &effects.status {
                text(status, 128)?;
            }
            if let Some(rows) = &effects.effects {
                if rows.len() > 512 {
                    return Err(SnapshotValidationError::TooLarge);
                }
                for row in rows {
                    for s in [&row.instance_key, &row.key, &row.name, &row.source] {
                        text(s, 512)?;
                    }
                    text(&row.description, 4096)?;
                    if ![row.duration, row.start_world_time, row.level]
                        .iter()
                        .all(|v| v.is_finite())
                    {
                        return Err(SnapshotValidationError::InvalidFormat);
                    }
                }
            }
        }
        // Include collection nodes, owned structs and identities with conservative headroom.
        for a in [&self.attacker_attributes, &self.victim_attributes]
            .into_iter()
            .flatten()
        {
            bytes += 1024 + a.values.as_ref().map_or(0, |v| v.len() * 256);
        }
        for e in [&self.attacker_effects, &self.victim_effects]
            .into_iter()
            .flatten()
        {
            bytes += 512
                + e.effects
                    .as_ref()
                    .map_or(0, |v| v.len() * std::mem::size_of::<Effect>());
        }
        if bytes > MAX_SNAPSHOT_BYTES {
            return Err(SnapshotValidationError::TooLarge);
        }
        Ok(bytes)
    }
    pub fn has_observations(&self) -> bool {
        [&self.attacker_attributes, &self.victim_attributes]
            .into_iter()
            .flatten()
            .any(|a| a.values.is_some())
            || [&self.attacker_effects, &self.victim_effects]
                .into_iter()
                .flatten()
                .any(|e| e.effects.is_some())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginHitSnapshot {
    /// Process-local immutable-object identity. Re-importing the same native key
    /// receives a new identity; never expose a pointer or reuse a prior source.
    #[serde(skip)]
    pub instance_id: u64,
    pub key: String,
    pub critical: Option<bool>,
    pub critical_source: Option<String>,
    pub role_effects: Option<EffectCounts>,
    pub enemy_effects: Option<EffectCounts>,
    pub retention: String,
    pub data: Option<HitSnapshotData>,
}
impl PluginHitSnapshot {
    pub fn reference(&self) -> String {
        self.instance_id.to_string()
    }
    pub fn seal(mut self) -> Result<Arc<Self>, SnapshotValidationError> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        self.validate()?;
        self.instance_id = NEXT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| SnapshotValidationError::InvalidFormat)?;
        Ok(Arc::new(self))
    }

    pub fn validate(&self) -> Result<(), SnapshotValidationError> {
        if self.key.is_empty()
            || self.key.len() > 256
            || self.critical_source.as_ref().is_some_and(|v| v.len() > 256)
        {
            return Err(SnapshotValidationError::InvalidFormat);
        }
        if !matches!(
            self.retention.as_str(),
            "available" | "missing" | "budget_exceeded"
        ) || (self.retention == "available") != self.data.is_some()
        {
            return Err(SnapshotValidationError::InvalidFormat);
        }
        for c in [self.role_effects, self.enemy_effects]
            .into_iter()
            .flatten()
        {
            if c.positive > 512
                || c.negative > 512
                || c.other > 512
                || c.positive + c.negative + c.other > 512
            {
                return Err(SnapshotValidationError::InvalidFormat);
            }
        }
        if let Some(data) = &self.data {
            data.validate()?;
        }
        Ok(())
    }
}
pub fn serialize<S: Serializer>(
    value: &Option<Arc<PluginHitSnapshot>>,
    s: S,
) -> Result<S::Ok, S::Error> {
    value.as_deref().serialize(s)
}
pub fn deserialize<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Arc<PluginHitSnapshot>>, D::Error> {
    let value = Option::<PluginHitSnapshot>::deserialize(d)?;
    if let Some(snapshot) = &value {
        snapshot.validate().map_err(serde::de::Error::custom)?;
    }
    value
        .map(|snapshot| snapshot.seal().map_err(serde::de::Error::custom))
        .transpose()
}
