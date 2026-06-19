#![allow(dead_code)]

use crate::model::Hit;
use crate::model::stable_hit_uid;

#[derive(Clone, Debug)]
pub struct DamageHitFact {
    pub hit_uid: String,
    pub timestamp: f64,
    pub char_id: u32,
    pub damage: f64,
    pub hp_before: f64,
    pub hp_after: f64,
    pub hp_reported_max: f64,
    pub target_context: Vec<String>,
}

impl From<&Hit> for DamageHitFact {
    fn from(hit: &Hit) -> Self {
        Self {
            hit_uid: stable_hit_uid(hit),
            timestamp: hit.timestamp,
            char_id: hit.char_id,
            damage: hit.damage,
            hp_before: hit.target_hp_before,
            hp_after: hit.target_hp_after,
            hp_reported_max: hit.target_max_hp,
            target_context: hit.target_context.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct HpUpdateFact {
    pub timestamp: f64,
    pub handle_key: String,
    pub current_hp: f64,
    pub max_hp: Option<f64>,
    pub evidence: String,
}

#[derive(Clone, Debug)]
pub struct RuntimeAliasFact {
    pub timestamp: f64,
    pub alias_key: String,
    pub target_path: Option<String>,
    pub target_name: Option<String>,
    pub evidence: String,
}

#[derive(Clone, Debug)]
pub struct TargetPathFact {
    pub timestamp: f64,
    pub target_path: String,
    pub target_name: Option<String>,
    pub evidence: String,
}

#[derive(Clone, Debug)]
pub enum LifecycleFact {
    Spawn {
        timestamp: f64,
        alias_key: Option<String>,
        target_path: Option<String>,
    },
    Death {
        timestamp: f64,
        hit_uid: Option<String>,
        hp_handle: Option<String>,
    },
    CloseAlias {
        timestamp: f64,
        alias_key: String,
    },
    Expire {
        timestamp: f64,
        target_track_id: String,
    },
}
