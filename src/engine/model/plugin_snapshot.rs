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

/// Native u64 identity or microsecond timestamp, kept as decimal text because it
/// exceeds JavaScript's safe integer range.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "tauri_frontend", derive(ts_rs::TS))]
pub struct SnapshotId(String);
impl<'de> Deserialize<'de> for SnapshotId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Id {
            Number(u64),
            Text(String),
        }
        let value = match Id::deserialize(d)? {
            Id::Number(n) => n,
            Id::Text(s) if s.bytes().all(|c| c.is_ascii_digit()) => s
                .parse()
                .map_err(|_| serde::de::Error::custom("invalid snapshot identity"))?,
            Id::Text(_) => return Err(serde::de::Error::custom("invalid snapshot identity")),
        };
        Ok(Self(value.to_string()))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "tauri_frontend", derive(ts_rs::TS))]
#[serde(untagged)]
pub enum AttributeValue {
    Number(f64),
    Boolean(bool),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "tauri_frontend", derive(ts_rs::TS))]
pub struct AttributeDatum {
    pub value: Option<AttributeValue>,
    pub status: String,
    pub source: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "tauri_frontend", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AttributeSnapshot {
    pub actor_index: i32,
    pub actor_serial: i32,
    #[serde(default)]
    pub id: Option<SnapshotId>,
    #[serde(default)]
    pub generation: Option<SnapshotId>,
    #[serde(default)]
    pub sampled_unix_us: Option<SnapshotId>,
    #[serde(default)]
    pub finished_unix_us: Option<SnapshotId>,
    #[serde(default)]
    pub actor_name: Option<String>,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub values: Option<BTreeMap<String, AttributeDatum>>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "tauri_frontend", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum EffectModifierOperation {
    AddBase,
    MultiplyAdditive,
    DivideAdditive,
    Override,
    MultiplyCompound,
    AddFinal,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "tauri_frontend", derive(ts_rs::TS))]
pub struct EffectModifier {
    pub attribute: String,
    pub operation: EffectModifierOperation,
    pub magnitude: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "tauri_frontend", derive(ts_rs::TS))]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum EffectModifiers {
    Recorded { values: Vec<EffectModifier> },
    Unavailable,
}
impl EffectModifiers {
    pub(crate) fn values(&self) -> Option<&[EffectModifier]> {
        match self {
            Self::Recorded { values } => Some(values),
            Self::Unavailable => None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "tauri_frontend", derive(ts_rs::TS))]
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
    // Frozen FModifierSpec observations; None only for older snapshots without this field.
    #[serde(default)]
    pub modifiers: Option<EffectModifiers>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "tauri_frontend", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct EffectSnapshot {
    pub actor_index: i32,
    pub actor_serial: i32,
    #[serde(default)]
    pub id: Option<SnapshotId>,
    #[serde(default)]
    pub observed_us: Option<SnapshotId>,
    #[serde(default)]
    pub complete: Option<bool>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub effects: Option<Vec<Effect>>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "tauri_frontend", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "tauri_frontend", derive(ts_rs::TS))]
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
                    for modifier in row
                        .modifiers
                        .as_ref()
                        .and_then(EffectModifiers::values)
                        .into_iter()
                        .flatten()
                    {
                        text(&modifier.attribute, 512)?;
                        if !modifier.magnitude.is_finite() {
                            return Err(SnapshotValidationError::InvalidFormat);
                        }
                    }
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
            if let Some(rows) = &e.effects {
                bytes += rows
                    .iter()
                    .filter_map(|row| row.modifiers.as_ref().and_then(EffectModifiers::values))
                    .map(std::mem::size_of_val)
                    .sum::<usize>();
            }
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
#[cfg_attr(feature = "tauri_frontend", derive(ts_rs::TS))]
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
