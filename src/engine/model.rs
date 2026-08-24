use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write as _;

const ABYSS_RESTART_STAGE_WINDOW_SECONDS: f64 = 10.0;

use serde::{Deserialize, Serialize};

/// Follow-up and server-correction producers retain at most 256 pending hits.
/// Keep twice that window so a reliable event burst can overtake newly decoded
/// hits without turning a single mutation into a scan of the complete combat
/// hits. Records store immutable locators and a few source aliases, so later
/// corrections can still find a hit after an earlier correction changed its
/// damage/HP fields.
const RECENT_HIT_MUTATION_WINDOW: usize = 512;
const OVERKILL_INTERVAL_WINDOW_SECONDS: f64 = 0.5;
const RECENT_HIT_SOURCE_ALIASES: usize = 4;
const MAX_DEBUG_PACKETS: usize = 10_000;
/// Debug packets are an optional diagnostic read model; raw PCAPNG is the
/// authoritative export. Bound both count and retained heap bytes so an
/// explicit FullDebug session cannot accumulate gigabytes of hex/text.
const MAX_DEBUG_PACKET_BYTES: usize = 16 * 1024 * 1024;
/// Character aggregates are a hot read model, not the authoritative hit log.
/// Reserve one row for every excess/hostile character id so HUD/readout work
/// remains bounded while team totals and attribution sums remain exact.
pub const MAX_COMBAT_STAT_ROWS: usize = 256;
pub const COMBAT_STATS_OVERFLOW_CHARACTER_ID: u32 = u32::MAX;
/// The timeline/history contract may expose at most this many pause bands.
/// Two slots are reserved for the compacted prefix and a currently active
/// pause, so projection remains bounded without dropping the exact full-span
/// frozen-duration total.
const MAX_PROJECTED_TIME_STOP_INTERVALS: usize = 4_096;
const MAX_RETAINED_TIME_STOP_INTERVALS: usize = MAX_PROJECTED_TIME_STOP_INTERVALS - 2;
const MAX_RETAINED_TIME_STOP_EVENTS: usize = 8_192;
const TIME_STOP_EVENT_COMPACTION_CHUNK: usize = 1_024;

#[cfg(test)]
std::thread_local! {
    static COMBAT_TOTAL_REBUILD_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static COMPACT_TIMELINE_PROJECTION_VISITS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static COMPACT_TIMELINE_ROLE_SCRATCH_SLOTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static COMPACT_TIMELINE_ROLE_CANDIDATE_SLOTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static COMBAT_DETAIL_QUERY_VISITS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static BULK_COMBAT_REBUILD_VISITS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(all(test, feature = "cli"))]
std::thread_local! {
    static GLOBAL_HIT_AXIS_PAGE_VISITS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(all(test, feature = "cli"))]
pub(crate) fn reset_global_hit_axis_page_visits() {
    GLOBAL_HIT_AXIS_PAGE_VISITS.with(|visits| visits.set(0));
}

#[cfg(all(test, feature = "cli"))]
pub(crate) fn global_hit_axis_page_visits() -> usize {
    GLOBAL_HIT_AXIS_PAGE_VISITS.with(std::cell::Cell::get)
}

/// Compact, exportable team DPS snapshot used to predict abyss clear time.
/// Deliberately holds no packets or per-hit data — only the latest total DPS and
/// up to 4 members — so the exported file stays tiny.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TeamDps {
    pub dps: f64,
    #[serde(default)]
    pub members: Vec<TeamDpsMember>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TeamDpsMember {
    pub id: u32,
    pub dps: f64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

/// On-disk "team DPS data" file: a single team and/or the abyss upper/lower
/// teams. Every field is optional so the same format covers single-team and
/// dual-team (abyss) exports. Serialized compactly (no pretty-printing).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TeamDpsExport {
    #[serde(default = "team_dps_export_version")]
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub single: Option<TeamDps>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upper: Option<TeamDps>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lower: Option<TeamDps>,
}

pub const TEAM_DPS_EXPORT_VERSION: u32 = 1;
pub const TEAM_DPS_MAX_MEMBERS: usize = 4;
pub const TEAM_DPS_MAX_MEMBER_NAME_BYTES: usize = 256;

fn team_dps_export_version() -> u32 {
    TEAM_DPS_EXPORT_VERSION
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CharacterInfo {
    #[serde(default)]
    pub name_zh: String,
    #[serde(default)]
    pub name_en: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub attribute: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HitCharacterSource {
    Packet,
    Session,
    GameplayEffect,
    ExportJson,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HitDirection {
    Outgoing,
    Incoming,
    #[default]
    Unknown,
}

impl HitDirection {
    pub const fn is_incoming(self) -> bool {
        matches!(self, Self::Incoming)
    }

    pub const fn is_outgoing(self) -> bool {
        matches!(self, Self::Outgoing)
    }

    pub const fn is_unknown(self) -> bool {
        matches!(self, Self::Unknown)
    }
}

impl TryFrom<&str> for HitDirection {
    type Error = &'static str;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "outgoing" => Ok(Self::Outgoing),
            "incoming" => Ok(Self::Incoming),
            "unknown" => Ok(Self::Unknown),
            _ => Err("invalid hit direction"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hit {
    pub timestamp: f64,
    pub char_id: u32,
    pub char_name: String,
    pub char_known: bool,
    pub damage: f64,
    pub byte_offset: usize,
    pub bit_shift: u8,
    pub char_source: HitCharacterSource,
    pub direction: HitDirection,
    pub target_hp_before: f64,
    pub target_hp_after: f64,
    pub target_max_hp: f64,
    #[serde(default)]
    pub max_hp_reduction: f64,
    pub target_hp_percent: f64,
    #[serde(default)]
    pub target_id: Option<String>,
    #[serde(default)]
    pub target_name: Option<String>,
    #[serde(default)]
    pub target_name_en: Option<String>,
    #[serde(default)]
    pub target_name_ja: Option<String>,
    #[serde(default)]
    pub target_monster_id: Option<String>,
    #[serde(default)]
    pub target_context: Vec<String>,
    #[serde(default)]
    pub gameplay_effect_index: Option<u32>,
    #[serde(default)]
    pub gameplay_effect_name: Option<String>,
    #[serde(default)]
    pub ability_name: Option<String>,
    #[serde(default)]
    pub damage_name: Option<String>,
    #[serde(default)]
    pub damage_component: Option<String>,
    #[serde(default)]
    pub attack_type: Option<String>,
    #[serde(default)]
    pub damage_attribute: Option<String>,
    #[serde(default)]
    pub follow_up_damage: f64,
    #[serde(default)]
    pub follow_up_timestamp: Option<f64>,
    #[serde(default)]
    pub follow_up_damage_name: Option<String>,
    #[serde(default)]
    pub follow_up_attack_type: Option<String>,
    #[serde(default)]
    pub follow_up_damage_attribute: Option<String>,
    /// Overkill reconciled across damage records that share the same target
    /// HP snapshot. `None` preserves the legacy per-hit derivation for older
    /// History/JSON records that predate interval reconciliation.
    #[serde(default)]
    pub reconciled_overkill_damage: Option<f64>,
    /// Parser-only identity of the SDK damage record. Capture reconciliation
    /// uses this to collapse the same wire event even when two attribution
    /// paths derive different target handles. It is intentionally not part of
    /// History or the external battle contract.
    #[serde(skip)]
    pub wire_event: Option<DamageWireEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DamageWireEvent {
    pub damage: f32,
    pub target_hp_before: f32,
    pub target_max_hp: f32,
    pub damage_time: f64,
    pub world_time: f32,
    pub repeated_damage: f32,
    pub state_flags: [i32; 3],
    pub trailing_value: f32,
}

impl Hit {
    pub fn total_damage(&self) -> f64 {
        self.damage + self.follow_up_damage
    }

    pub fn overkill_damage(&self) -> f64 {
        // Overkill starts only after the target reaches its terminal HP state.
        // This read-side invariant also repairs older persisted rows whose
        // interval estimate was recorded before a later server HP correction.
        if self.target_max_hp > 0.0
            && self.target_hp_after.is_finite()
            && self.target_hp_after > 0.0
        {
            return 0.0;
        }
        if let Some(overkill) = self
            .reconciled_overkill_damage
            .filter(|value| value.is_finite())
        {
            return overkill.clamp(0.0, self.damage.max(0.0));
        }
        if self.target_max_hp <= 0.0
            || !self.damage.is_finite()
            || !self.target_hp_before.is_finite()
        {
            return 0.0;
        }
        (self.damage - self.target_hp_before).max(0.0)
    }

    pub fn is_server_damage_reconciliation(&self) -> bool {
        self.char_source == HitCharacterSource::Unknown
            && self.char_id == 0
            && self.direction.is_outgoing()
            && self.damage_name.as_deref() == Some("Server settlement residual")
            && self.target_id.is_some()
    }
}

fn reconcile_latest_overkill_interval(hits: &mut VecDeque<Hit>) {
    let Some(latest) = hits.back() else {
        return;
    };
    let Some(latest_wire) = latest.wire_event else {
        return;
    };
    let Some(target_id) = latest.target_id.clone() else {
        return;
    };
    if !latest.direction.is_outgoing()
        || !latest_wire.target_hp_before.is_finite()
        || latest_wire.target_hp_before <= 0.0
        || !latest_wire.target_max_hp.is_finite()
        || latest_wire.target_max_hp <= 0.0
        || !latest_wire.damage_time.is_finite()
    {
        return;
    }

    let latest_damage_time = latest_wire.damage_time;
    let mut interval = hits
        .iter()
        .enumerate()
        .rev()
        .take(RECENT_HIT_MUTATION_WINDOW)
        .filter_map(|(index, candidate)| {
            let wire = candidate.wire_event?;
            (candidate.direction.is_outgoing()
                && candidate.target_id.as_deref() == Some(target_id.as_str())
                && (wire.target_hp_before - latest_wire.target_hp_before).abs() <= 0.5
                && (wire.target_max_hp - latest_wire.target_max_hp).abs() <= 0.5
                && wire.damage_time.is_finite()
                && (wire.damage_time - latest_damage_time).abs()
                    <= OVERKILL_INTERVAL_WINDOW_SECONDS)
                .then_some((index, wire.damage_time))
        })
        .collect::<Vec<_>>();
    interval.sort_by(|(left_index, left_time), (right_index, right_time)| {
        left_time
            .total_cmp(right_time)
            .then_with(|| left_index.cmp(right_index))
    });

    let mut remaining_hp = f64::from(latest_wire.target_hp_before);
    for (index, _) in interval {
        let Some(hit) = hits.get_mut(index) else {
            continue;
        };
        let damage = hit.damage.max(0.0);
        hit.reconciled_overkill_damage = Some((damage - remaining_hp).max(0.0));
        remaining_hp = (remaining_hp - damage).max(0.0);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HitFollowUp {
    pub source_timestamp: f64,
    pub source_char_id: u32,
    pub source_damage: f64,
    pub source_target_hp_before: f64,
    pub source_target_hp_after: f64,
    pub source_target_max_hp: f64,
    #[serde(default)]
    pub source_gameplay_effect_index: Option<u32>,
    pub timestamp: f64,
    pub damage: f64,
    pub target_hp_after: f64,
    pub target_hp_percent: f64,
    #[serde(default)]
    pub damage_name: Option<String>,
    #[serde(default)]
    pub attack_type: Option<String>,
    #[serde(default)]
    pub damage_attribute: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HitDamageCorrection {
    pub source_timestamp: f64,
    pub source_char_id: u32,
    pub source_damage: f64,
    pub source_target_hp_before: f64,
    pub source_target_hp_after: f64,
    pub source_target_max_hp: f64,
    #[serde(default)]
    pub source_gameplay_effect_index: Option<u32>,
    pub damage: f64,
    pub target_hp_before: f64,
    pub target_hp_after: f64,
    pub target_hp_percent: f64,
    /// `Some` updates the maximum-HP reduction attributed to this hit; `None`
    /// preserves older capture/history behavior.
    #[serde(default)]
    pub max_hp_reduction: Option<f64>,
    /// `Some` is an authoritative server settlement; `None` leaves any
    /// previously reconciled overkill label unchanged.
    #[serde(default)]
    pub reconciled_overkill_damage: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct PacketDebug {
    pub timestamp: f64,
    pub source: String,
    pub destination: String,
    pub direction: String,
    pub payload_len: usize,
    pub declared_ids: Vec<u32>,
    pub parsed_hits: usize,
    pub note: String,
    pub payload_preview: String,
    pub payload_hex: String,
    pub decoded_text: String,
}

impl PacketDebug {
    fn retained_heap_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(self.source.capacity())
            .saturating_add(self.destination.capacity())
            .saturating_add(self.direction.capacity())
            .saturating_add(
                self.declared_ids
                    .capacity()
                    .saturating_mul(std::mem::size_of::<u32>()),
            )
            .saturating_add(self.note.capacity())
            .saturating_add(self.payload_preview.capacity())
            .saturating_add(self.payload_hex.capacity())
            .saturating_add(self.decoded_text.capacity())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketObservation {
    pub parsed_hits: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HtItemNetId {
    pub solt: u32,
    pub serial: u32,
}

impl HtItemNetId {
    pub const ZERO: Self = Self { solt: 0, serial: 0 };

    pub fn is_zero(self) -> bool {
        self == Self::ZERO
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EquipmentStat {
    pub property: String,
    pub value: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmptyCurtainPlacement {
    pub row: i32,
    pub column: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EmptyCurtainItem {
    pub id: HtItemNetId,
    pub item_id: String,
    pub level: u32,
    #[serde(default)]
    pub main_stats: Vec<EquipmentStat>,
    #[serde(default)]
    pub sub_stats: Vec<EquipmentStat>,
    pub locked: bool,
    #[serde(default)]
    pub discarded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_net_id: Option<HtItemNetId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equipped_character_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equipped_placement: Option<EmptyCurtainPlacement>,
}

impl EmptyCurtainItem {
    pub fn is_equipped(&self) -> bool {
        self.character_net_id.is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmptyCurtainCharacter {
    pub net_id: HtItemNetId,
    pub character_id: u32,
}

#[derive(Clone, Debug, Default)]
pub struct CharacterStats {
    pub char_id: u32,
    pub name: String,
    pub hits: u64,
    pub damage: f64,
    pub attributed_hits: u64,
    pub attributed_damage: f64,
    pub attributed_first_hit: Option<f64>,
    pub attributed_last_hit: Option<f64>,
    pub direct_hits: u64,
    pub direct_damage: f64,
    pub direct_first_hit: Option<f64>,
    pub direct_last_hit: Option<f64>,
    pub hits_taken: u64,
    pub damage_taken: f64,
    pub first_hit: f64,
    pub last_hit: f64,
    /// Number of retained hits that make this character eligible for compact
    /// HUD rows. Keeping this alongside the other authoritative aggregates
    /// avoids rebuilding a character-id set by scanning an unbounded combat on
    /// every desktop refresh. It is intentionally not a serialized contract.
    hud_visible_hits: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DamageAttributionSummary {
    pub total_damage: f64,
    pub max_hp_reduction: f64,
    pub character_direct_damage: f64,
    pub character_reaction_damage: f64,
    pub shared_damage: f64,
    pub unattributed_damage: f64,
}

impl DamageAttributionSummary {
    pub fn character_damage(self, separate_reaction_damage: bool) -> f64 {
        self.character_direct_damage
            + if separate_reaction_damage {
                0.0
            } else {
                self.character_reaction_damage
            }
    }

    pub fn with_max_hp_reduction_in_total(mut self, include: bool) -> Self {
        if include {
            self.total_damage += self.max_hp_reduction;
        }
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HitDirectionSummary {
    pub outgoing_damage: f64,
    pub outgoing_hits: u64,
    pub unknown_damage: f64,
    pub unknown_hits: u64,
    pub incoming_damage: f64,
    pub incoming_hits: u64,
}

impl HitDirectionSummary {
    pub fn unknown_share(&self) -> f64 {
        let total_output = self.outgoing_damage + self.unknown_damage;
        if total_output > 0.0 {
            self.unknown_damage / total_output * 100.0
        } else {
            0.0
        }
    }
}

pub fn summarize_hit_directions<'a>(
    hits: impl IntoIterator<Item = &'a Hit>,
) -> HitDirectionSummary {
    let mut summary = HitDirectionSummary::default();
    for hit in hits {
        let damage = hit.total_damage();
        match hit.direction {
            HitDirection::Incoming => {
                summary.incoming_damage += damage;
                summary.incoming_hits += 1;
            }
            HitDirection::Outgoing => {
                summary.outgoing_damage += damage;
                summary.outgoing_hits += 1;
            }
            HitDirection::Unknown => {
                summary.unknown_damage += damage;
                summary.unknown_hits += 1;
            }
        }
    }
    summary
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TimelineTimeStopInterval {
    pub start_offset: f64,
    pub end_offset: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimelineRoleBucket {
    pub char_id: u32,
    pub char_name: String,
    pub damage: f64,
    pub dps: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimelineBucket {
    pub start_offset: f64,
    pub end_offset: f64,
    pub damage: f64,
    pub dps: f64,
    pub cumulative_damage: f64,
    pub hits: u64,
    pub role_damage: Vec<TimelineRoleBucket>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimelineMarkerKind {
    HalfStart,
    Clear,
    Exit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineMarker {
    pub offset: f64,
    pub label: String,
    pub kind: TimelineMarkerKind,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimelineSeries {
    pub bucket_seconds: f64,
    pub start_timestamp: Option<f64>,
    pub end_timestamp: Option<f64>,
    pub total_damage: f64,
    /// Damage/hits retained in team buckets but omitted from per-role rows
    /// because the caller's role/character budget was exhausted.
    pub omitted_role_damage: f64,
    pub omitted_role_hits: u64,
    pub buckets: Vec<TimelineBucket>,
    pub time_stop_intervals: Vec<TimelineTimeStopInterval>,
    /// Number of old authoritative pause intervals represented by the single
    /// density-projected prefix band. Zero means every returned band is exact.
    pub compacted_time_stop_intervals: u64,
    pub markers: Vec<TimelineMarker>,
}

/// Source-side per-role aggregate for the compact desktop timeline.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompactTimelineRoleBucket {
    pub char_id: u32,
    pub char_name: String,
    pub damage: f64,
    pub hits: u64,
}

/// Source-side aggregate for a compact timeline bucket. The result is bounded
/// by the caller before allocation; authoritative hits remain intact.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompactTimelineBucket {
    pub start_offset: f64,
    pub end_offset: f64,
    pub damage: f64,
    pub hits: u64,
    pub roles: Vec<CompactTimelineRoleBucket>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompactTimelineSeries {
    pub bucket_seconds: f64,
    pub start_timestamp: Option<f64>,
    pub end_timestamp: Option<f64>,
    pub duration_seconds: f64,
    pub omitted_role_damage: f64,
    pub omitted_role_hits: u64,
    pub buckets: Vec<CompactTimelineBucket>,
}

/// O(1) source-side cardinality metadata for timeline role admission.
/// `retained` is exact while `overflowed` is false; once the fixed role index
/// overflows, callers must treat the real distinct count as greater than the
/// retained source budget instead of rescanning authoritative hits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BoundedTimelineCharacterCount {
    pub retained: usize,
    pub overflowed: bool,
}

impl BoundedTimelineCharacterCount {
    pub const fn exceeds(self, limit: usize) -> bool {
        self.retained > limit || (self.overflowed && self.retained >= limit)
    }
}

#[derive(Clone, Copy, Debug)]
struct CompactTimelineLayout {
    base_bucket_seconds: f64,
    base_bucket_count: usize,
    group_size: usize,
    bucket_seconds: f64,
    bucket_count: usize,
}

impl CompactTimelineLayout {
    fn new(
        start: f64,
        end: f64,
        requested_bucket_seconds: f64,
        max_buckets: usize,
    ) -> Option<Self> {
        if !start.is_finite() || !end.is_finite() || max_buckets == 0 {
            return None;
        }
        let base_bucket_seconds =
            if requested_bucket_seconds.is_finite() && requested_bucket_seconds > 0.0 {
                requested_bucket_seconds
            } else {
                1.0
            };
        let raw_span = end - start;
        let span = if raw_span.is_finite() {
            raw_span.max(0.0)
        } else {
            f64::MAX
        };
        let ratio = span / base_bucket_seconds;
        if !ratio.is_finite() || ratio >= usize::MAX as f64 {
            let bucket_seconds = (span / max_buckets as f64).max(base_bucket_seconds);
            return Some(Self {
                base_bucket_seconds: bucket_seconds,
                base_bucket_count: max_buckets,
                group_size: 1,
                bucket_seconds,
                bucket_count: max_buckets,
            });
        }
        let base_bucket_count = (ratio.floor() as usize).saturating_add(1);
        let group_size = base_bucket_count.div_ceil(max_buckets).max(1);
        let bucket_count = base_bucket_count.div_ceil(group_size);
        let grouped_bucket_seconds = base_bucket_seconds * group_size as f64;
        let bucket_seconds = if grouped_bucket_seconds.is_finite() {
            grouped_bucket_seconds
        } else {
            f64::MAX
        };
        Some(Self {
            base_bucket_seconds,
            base_bucket_count,
            group_size,
            bucket_seconds,
            bucket_count,
        })
    }

    fn bucket_index(self, offset: f64) -> usize {
        let base_index = ((offset.max(0.0) / self.base_bucket_seconds).floor() as usize)
            .min(self.base_bucket_count.saturating_sub(1));
        (base_index / self.group_size).min(self.bucket_count.saturating_sub(1))
    }

    fn bucket_index_for_timestamp(self, timestamp: f64, start: f64, end: f64) -> usize {
        let raw_offset = timestamp - start;
        if raw_offset.is_finite() {
            return self.bucket_index(raw_offset);
        }

        // Subtraction can overflow even though all timestamps are finite.
        // Halving first preserves the legacy projection's fallback mapping.
        let scaled_span = end / 2.0 - start / 2.0;
        let relative = if scaled_span.is_finite() && scaled_span > 0.0 {
            ((timestamp / 2.0 - start / 2.0) / scaled_span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        ((relative * self.bucket_count as f64).floor() as usize)
            .min(self.bucket_count.saturating_sub(1))
    }

    fn single_bucket_for_range(
        self,
        min_timestamp: f64,
        max_timestamp: f64,
        start: f64,
        end: f64,
    ) -> Option<usize> {
        let min_offset_is_finite = (min_timestamp - start).is_finite();
        let max_offset_is_finite = (max_timestamp - start).is_finite();
        if min_offset_is_finite != max_offset_is_finite {
            // The legacy extreme-range mapping switches from direct offsets to
            // its scaled fallback at this boundary, so endpoint equality alone
            // cannot prove that every timestamp between them shares a bucket.
            return None;
        }
        let first = self.bucket_index_for_timestamp(min_timestamp, start, end);
        let last = self.bucket_index_for_timestamp(max_timestamp, start, end);
        (first == last).then_some(first)
    }

    fn new_timeline(
        start: f64,
        end: f64,
        requested_bucket_seconds: f64,
        max_buckets: usize,
    ) -> Option<Self> {
        if !start.is_finite() || !end.is_finite() || max_buckets == 0 {
            return None;
        }
        let requested_bucket_seconds =
            if requested_bucket_seconds.is_finite() && requested_bucket_seconds > 0.0 {
                requested_bucket_seconds
            } else {
                1.0
            };
        let raw_span = end - start;
        let span = if raw_span.is_finite() {
            raw_span.max(0.0)
        } else {
            f64::MAX
        };
        let ratio = span / requested_bucket_seconds;
        let requested_count = if !ratio.is_finite() || ratio >= usize::MAX as f64 {
            usize::MAX
        } else {
            (ratio.floor() as usize).saturating_add(1)
        };
        let (bucket_seconds, bucket_count) = if requested_count <= max_buckets {
            (requested_bucket_seconds, requested_count)
        } else {
            (
                (span / max_buckets as f64).max(requested_bucket_seconds),
                max_buckets,
            )
        };
        Some(Self {
            base_bucket_seconds: bucket_seconds,
            base_bucket_count: bucket_count,
            group_size: 1,
            bucket_seconds,
            bucket_count,
        })
    }

    fn bucket_bounds(self, index: usize) -> (f64, f64) {
        let first_base_bucket = index.saturating_mul(self.group_size);
        let end_base_bucket = index
            .saturating_add(1)
            .saturating_mul(self.group_size)
            .min(self.base_bucket_count);
        (
            finite_timeline_offset(first_base_bucket, self.base_bucket_seconds),
            finite_timeline_offset(end_base_bucket, self.base_bucket_seconds),
        )
    }
}

const COMPACT_TIMELINE_MAX_CHARACTERS: usize = 256;
const COMPACT_TIMELINE_MAX_TREE_DEPTH: usize = u64::BITS as usize;

#[derive(Clone, Copy, Debug, PartialEq)]
struct CompactTimelineAggregate {
    damage: f64,
    hits: u64,
    first_sequence: u64,
}

#[derive(Clone, Copy, Debug)]
struct CompactTimelineAdjustment {
    key: u64,
    timestamp: f64,
    damage_delta: f64,
    hits_delta: i8,
    sequence: u64,
}

impl Default for CompactTimelineAggregate {
    fn default() -> Self {
        Self {
            damage: 0.0,
            hits: 0,
            first_sequence: u64::MAX,
        }
    }
}

impl CompactTimelineAggregate {
    fn combine(self, other: Self) -> Self {
        Self {
            damage: self.damage + other.damage,
            hits: self.hits.saturating_add(other.hits),
            first_sequence: self.first_sequence.min(other.first_sequence),
        }
    }

    fn adjust(&mut self, damage_delta: f64, hits_delta: i8, sequence: u64) {
        self.damage += damage_delta;
        if self.damage.abs() <= 1e-9 {
            self.damage = 0.0;
        }
        if hits_delta >= 0 {
            self.hits = self.hits.saturating_add(hits_delta as u64);
            if hits_delta > 0 {
                self.first_sequence = self.first_sequence.min(sequence);
            }
        } else {
            self.hits = self.hits.saturating_sub((-hits_delta) as u64);
            if self.hits == 0 {
                self.first_sequence = u64::MAX;
            }
        }
    }
}

/// Arena-backed crit-bit tree. Branch bits strictly decrease on every path,
/// so lookup depth is hard-bounded to 64 for adversarial replay timestamps.
/// Updates, projection, cloning, and drop are all iterative/non-recursive.
#[derive(Clone, Debug, Default)]
struct CompactTimelineTree {
    root: Option<usize>,
    nodes: Vec<CompactTimelineTreeNode>,
}

#[derive(Clone, Debug)]
enum CompactTimelineTreeNode {
    Leaf {
        key: u64,
        timestamp: f64,
        aggregate: CompactTimelineAggregate,
    },
    Branch {
        critical_bit: u8,
        left: usize,
        right: usize,
        aggregate: CompactTimelineAggregate,
        min_timestamp: f64,
        max_timestamp: f64,
    },
}

impl CompactTimelineTreeNode {
    fn summary(&self) -> (CompactTimelineAggregate, f64, f64) {
        match self {
            Self::Leaf {
                timestamp,
                aggregate,
                ..
            } => (*aggregate, *timestamp, *timestamp),
            Self::Branch {
                aggregate,
                min_timestamp,
                max_timestamp,
                ..
            } => (*aggregate, *min_timestamp, *max_timestamp),
        }
    }
}

impl CompactTimelineTree {
    fn aggregate(&self) -> CompactTimelineAggregate {
        self.root
            .and_then(|root| self.nodes.get(root))
            .map(|node| node.summary().0)
            .unwrap_or_default()
    }

    fn adjust(&mut self, timestamp: f64, damage_delta: f64, hits_delta: i8, sequence: u64) {
        let adjustment = CompactTimelineAdjustment {
            key: compact_timeline_ordered_key(timestamp),
            timestamp,
            damage_delta,
            hits_delta,
            sequence,
        };
        let Some(root) = self.root else {
            self.nodes.push(CompactTimelineTreeNode::Leaf {
                key: adjustment.key,
                timestamp,
                aggregate: CompactTimelineAggregate {
                    damage: damage_delta,
                    hits: hits_delta.max(0) as u64,
                    first_sequence: sequence,
                },
            });
            self.root = Some(0);
            return;
        };

        let mut cursor = root;
        let mut search_path = [0_usize; COMPACT_TIMELINE_MAX_TREE_DEPTH];
        let mut search_depth = 0_usize;
        for _ in 0..=COMPACT_TIMELINE_MAX_TREE_DEPTH {
            let Some(node) = self.nodes.get(cursor) else {
                return;
            };
            match node {
                CompactTimelineTreeNode::Leaf {
                    key: existing_key, ..
                } => {
                    if *existing_key == adjustment.key {
                        if let Some(CompactTimelineTreeNode::Leaf { aggregate, .. }) =
                            self.nodes.get_mut(cursor)
                        {
                            aggregate.adjust(damage_delta, hits_delta, sequence);
                        }
                        self.recalculate_path(&search_path[..search_depth]);
                        return;
                    }
                    self.insert_distinct(adjustment, *existing_key);
                    return;
                }
                CompactTimelineTreeNode::Branch {
                    critical_bit,
                    left,
                    right,
                    ..
                } => {
                    let Some(path_slot) = search_path.get_mut(search_depth) else {
                        return;
                    };
                    *path_slot = cursor;
                    search_depth += 1;
                    cursor = if compact_timeline_key_bit(adjustment.key, *critical_bit) {
                        *right
                    } else {
                        *left
                    };
                }
            }
        }
    }

    fn insert_distinct(&mut self, adjustment: CompactTimelineAdjustment, existing_key: u64) {
        let critical_bit = (u64::BITS - 1 - (adjustment.key ^ existing_key).leading_zeros()) as u8;
        let Some(mut cursor) = self.root else {
            return;
        };
        let mut parent = None::<(usize, bool)>;
        let mut ancestors = [0_usize; COMPACT_TIMELINE_MAX_TREE_DEPTH];
        let mut ancestor_count = 0_usize;
        for _ in 0..=COMPACT_TIMELINE_MAX_TREE_DEPTH {
            let Some(node) = self.nodes.get(cursor) else {
                return;
            };
            let CompactTimelineTreeNode::Branch {
                critical_bit: branch_bit,
                left,
                right,
                ..
            } = node
            else {
                break;
            };
            if *branch_bit <= critical_bit {
                break;
            }
            let Some(path_slot) = ancestors.get_mut(ancestor_count) else {
                return;
            };
            *path_slot = cursor;
            ancestor_count += 1;
            let right_side = compact_timeline_key_bit(adjustment.key, *branch_bit);
            parent = Some((cursor, right_side));
            cursor = if right_side { *right } else { *left };
        }

        let leaf_index = self.nodes.len();
        self.nodes.push(CompactTimelineTreeNode::Leaf {
            key: adjustment.key,
            timestamp: adjustment.timestamp,
            aggregate: CompactTimelineAggregate {
                damage: adjustment.damage_delta,
                hits: adjustment.hits_delta.max(0) as u64,
                first_sequence: adjustment.sequence,
            },
        });
        let new_goes_right = compact_timeline_key_bit(adjustment.key, critical_bit);
        let (left, right) = if new_goes_right {
            (cursor, leaf_index)
        } else {
            (leaf_index, cursor)
        };
        let branch_index = self.nodes.len();
        let Some((left_summary, left_min, left_max)) =
            self.nodes.get(left).map(CompactTimelineTreeNode::summary)
        else {
            return;
        };
        let Some((right_summary, right_min, right_max)) =
            self.nodes.get(right).map(CompactTimelineTreeNode::summary)
        else {
            return;
        };
        self.nodes.push(CompactTimelineTreeNode::Branch {
            critical_bit,
            left,
            right,
            aggregate: left_summary.combine(right_summary),
            min_timestamp: left_min.min(right_min),
            max_timestamp: left_max.max(right_max),
        });
        match parent {
            Some((parent_index, right_side)) => {
                if let Some(CompactTimelineTreeNode::Branch { left, right, .. }) =
                    self.nodes.get_mut(parent_index)
                {
                    if right_side {
                        *right = branch_index;
                    } else {
                        *left = branch_index;
                    }
                }
            }
            None => self.root = Some(branch_index),
        }
        self.recalculate_path(&ancestors[..ancestor_count]);
    }

    fn recalculate_path(&mut self, path: &[usize]) {
        for &index in path.iter().rev() {
            let Some((left, right)) = self.nodes.get(index).and_then(|node| match node {
                CompactTimelineTreeNode::Branch { left, right, .. } => Some((*left, *right)),
                CompactTimelineTreeNode::Leaf { .. } => None,
            }) else {
                continue;
            };
            let Some((left_summary, left_min, left_max)) =
                self.nodes.get(left).map(CompactTimelineTreeNode::summary)
            else {
                continue;
            };
            let Some((right_summary, right_min, right_max)) =
                self.nodes.get(right).map(CompactTimelineTreeNode::summary)
            else {
                continue;
            };
            if let Some(CompactTimelineTreeNode::Branch {
                aggregate,
                min_timestamp,
                max_timestamp,
                ..
            }) = self.nodes.get_mut(index)
            {
                *aggregate = left_summary.combine(right_summary);
                *min_timestamp = left_min.min(right_min);
                *max_timestamp = left_max.max(right_max);
            }
        }
    }

    fn distribute(
        &self,
        layout: CompactTimelineLayout,
        start: f64,
        end: f64,
        buckets: &mut [CompactTimelineAggregate],
        touched_buckets: Option<&mut Vec<usize>>,
    ) {
        let Some(root) = self.root else {
            return;
        };
        let mut touched_buckets = touched_buckets;
        let mut add_to_bucket = |bucket_index: usize, aggregate: CompactTimelineAggregate| {
            let Some(bucket) = buckets.get_mut(bucket_index) else {
                return;
            };
            let first_touch = bucket.hits == 0;
            *bucket = bucket.combine(aggregate);
            if first_touch
                && aggregate.hits > 0
                && let Some(touched) = &mut touched_buckets
            {
                touched.push(bucket_index);
            }
        };
        let mut pending = [0_usize; COMPACT_TIMELINE_MAX_TREE_DEPTH + 1];
        pending[0] = root;
        let mut pending_len = 1_usize;
        while pending_len > 0 {
            pending_len -= 1;
            let index = pending[pending_len];
            #[cfg(test)]
            COMPACT_TIMELINE_PROJECTION_VISITS
                .with(|visits| visits.set(visits.get().saturating_add(1)));
            let Some(node) = self.nodes.get(index) else {
                continue;
            };
            let (aggregate, min_timestamp, max_timestamp) = node.summary();
            if aggregate.hits == 0 {
                continue;
            }
            if let Some(bucket_index) =
                layout.single_bucket_for_range(min_timestamp, max_timestamp, start, end)
            {
                add_to_bucket(bucket_index, aggregate);
                continue;
            }
            match node {
                CompactTimelineTreeNode::Branch { left, right, .. } => {
                    let Some(next_len) = pending_len.checked_add(2) else {
                        return;
                    };
                    if next_len > pending.len() {
                        // The crit-bit depth invariant limits the DFS frontier
                        // to 65 entries. Fail closed rather than recurse/panic
                        // if internal state is ever corrupted.
                        return;
                    }
                    pending[pending_len] = *right;
                    pending[pending_len + 1] = *left;
                    pending_len = next_len;
                }
                CompactTimelineTreeNode::Leaf { .. } => {
                    let bucket_index = layout.bucket_index_for_timestamp(min_timestamp, start, end);
                    add_to_bucket(bucket_index, aggregate);
                }
            }
        }
    }

    #[cfg(test)]
    fn max_depth(&self) -> usize {
        let Some(root) = self.root else {
            return 0;
        };
        let mut maximum = 0;
        let mut pending = vec![(root, 0_usize)];
        while let Some((index, depth)) = pending.pop() {
            maximum = maximum.max(depth);
            if let Some(CompactTimelineTreeNode::Branch { left, right, .. }) = self.nodes.get(index)
            {
                pending.push((*left, depth.saturating_add(1)));
                pending.push((*right, depth.saturating_add(1)));
            }
        }
        maximum
    }
}

#[derive(Clone, Debug)]
struct CompactTimelineRoleIndex {
    name: String,
    tree: CompactTimelineTree,
}

#[derive(Clone, Copy, Debug)]
struct CompactTimelineRoleEvent {
    first_sequence: u64,
    char_id: u32,
    role_index: usize,
    aggregate: CompactTimelineAggregate,
}

impl CompactTimelineRoleEvent {
    fn admission_key(self) -> (u64, u32) {
        (self.first_sequence, self.char_id)
    }
}

/// Fixed logical slots for one output bucket. Buckets allocate this structure
/// lazily, and each structure has exactly the caller's per-bucket role budget,
/// so all candidate storage is hard-bounded by bucket_count * role_budget.
#[derive(Debug)]
struct CompactTimelineRoleCandidates {
    slots: Box<[Option<CompactTimelineRoleEvent>]>,
    len: usize,
}

impl CompactTimelineRoleCandidates {
    fn new(capacity: usize) -> Self {
        Self {
            slots: vec![None; capacity].into_boxed_slice(),
            len: 0,
        }
    }

    fn consider(&mut self, event: CompactTimelineRoleEvent) {
        if self.len < self.slots.len() {
            self.slots[self.len] = Some(event);
            self.len += 1;
            return;
        }

        let mut worst = None::<(usize, (u64, u32))>;
        for (index, slot) in self.slots.iter().enumerate() {
            let Some(retained) = slot else {
                continue;
            };
            let key = retained.admission_key();
            if worst.is_none_or(|(_, worst_key)| key > worst_key) {
                worst = Some((index, key));
            }
        }
        if let Some((index, worst_key)) = worst
            && event.admission_key() < worst_key
        {
            self.slots[index] = Some(event);
        }
    }
}

/// Exact, incrementally maintained team/per-role timestamp index. The team
/// tree and all role trees together contain fewer than four arena nodes per
/// retained hit; role identity count is capped at the source contract maximum.
#[derive(Clone, Debug, Default)]
struct CompactTimelineIndex {
    team: CompactTimelineTree,
    roles: HashMap<u32, CompactTimelineRoleIndex>,
    active_roles: usize,
    role_overflowed: bool,
    start: Option<f64>,
    end: Option<f64>,
    next_sequence: u64,
}

impl CompactTimelineIndex {
    fn observe_hit(&mut self, hit: &Hit) {
        if hit.direction.is_incoming() || !hit.timestamp.is_finite() {
            return;
        }
        self.start = Some(
            self.start
                .map_or(hit.timestamp, |value| value.min(hit.timestamp)),
        );
        self.end = Some(
            self.end
                .map_or(hit.timestamp, |value| value.max(hit.timestamp)),
        );
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        let damage = hit.total_damage();
        if !damage.is_finite() {
            return;
        }
        self.team.adjust(hit.timestamp, damage, 1, sequence);
        if let Some(role) = self.roles.get_mut(&hit.char_id) {
            let was_active = role.tree.aggregate().hits > 0;
            role.name.clone_from(&hit.char_name);
            role.tree.adjust(hit.timestamp, damage, 1, sequence);
            if !was_active && role.tree.aggregate().hits > 0 {
                self.active_roles = self.active_roles.saturating_add(1);
            }
        } else if self.roles.len() < COMPACT_TIMELINE_MAX_CHARACTERS {
            let mut tree = CompactTimelineTree::default();
            tree.adjust(hit.timestamp, damage, 1, sequence);
            self.roles.insert(
                hit.char_id,
                CompactTimelineRoleIndex {
                    name: hit.char_name.clone(),
                    tree,
                },
            );
            self.active_roles = self.active_roles.saturating_add(1);
        } else {
            self.role_overflowed = true;
        }
    }

    fn apply_mutation(
        &mut self,
        before: HitAggregateContribution,
        after: HitAggregateContribution,
    ) {
        let before_counted =
            !before.incoming && before.timestamp.is_finite() && before.total_damage.is_finite();
        let after_counted =
            !after.incoming && after.timestamp.is_finite() && after.total_damage.is_finite();
        let (timestamp, damage_delta, hits_delta) = match (before_counted, after_counted) {
            (true, true) => (after.timestamp, after.total_damage - before.total_damage, 0),
            (true, false) => (before.timestamp, -before.total_damage, -1),
            (false, true) => (after.timestamp, after.total_damage, 1),
            (false, false) => return,
        };
        self.team
            .adjust(timestamp, damage_delta, hits_delta, self.next_sequence);
        if let Some(role) = self.roles.get_mut(&after.char_id) {
            let was_active = role.tree.aggregate().hits > 0;
            role.tree
                .adjust(timestamp, damage_delta, hits_delta, self.next_sequence);
            let is_active = role.tree.aggregate().hits > 0;
            match (was_active, is_active) {
                (false, true) => self.active_roles = self.active_roles.saturating_add(1),
                (true, false) => self.active_roles = self.active_roles.saturating_sub(1),
                _ => {}
            }
        }
    }

    const fn bounded_character_count(&self) -> BoundedTimelineCharacterCount {
        BoundedTimelineCharacterCount {
            retained: self.active_roles,
            overflowed: self.role_overflowed,
        }
    }

    fn project(
        &self,
        requested_bucket_seconds: f64,
        max_buckets: usize,
        max_roles_per_bucket: usize,
        max_characters: usize,
    ) -> Option<CompactTimelineSeries> {
        let (start, end) = self.start.zip(self.end)?;
        let layout = CompactTimelineLayout::new(start, end, requested_bucket_seconds, max_buckets)?;
        Some(self.project_layout(start, end, layout, max_roles_per_bucket, max_characters))
    }

    fn project_timeline(
        &self,
        start: Option<f64>,
        end: Option<f64>,
        requested_bucket_seconds: f64,
        max_buckets: usize,
        max_roles_per_bucket: usize,
        max_characters: usize,
    ) -> Option<CompactTimelineSeries> {
        let (start, end) = start.zip(end)?;
        let layout =
            CompactTimelineLayout::new_timeline(start, end, requested_bucket_seconds, max_buckets)?;
        Some(self.project_layout(start, end, layout, max_roles_per_bucket, max_characters))
    }

    fn project_layout(
        &self,
        start: f64,
        end: f64,
        layout: CompactTimelineLayout,
        max_roles_per_bucket: usize,
        max_characters: usize,
    ) -> CompactTimelineSeries {
        let mut team_buckets = vec![CompactTimelineAggregate::default(); layout.bucket_count];
        self.team
            .distribute(layout, start, end, &mut team_buckets, None);
        let mut buckets = team_buckets
            .iter()
            .enumerate()
            .map(|(index, aggregate)| {
                let (start_offset, end_offset) = layout.bucket_bounds(index);
                CompactTimelineBucket {
                    start_offset,
                    end_offset,
                    damage: aggregate.damage,
                    hits: aggregate.hits,
                    roles: Vec::new(),
                }
            })
            .collect::<Vec<_>>();

        let character_budget = max_characters.min(COMPACT_TIMELINE_MAX_CHARACTERS);
        if character_budget > 0 && !self.roles.is_empty() {
            // Character admission is deterministic and independent of HashMap
            // order: retain the globally earliest roles first, then keep each
            // bucket's earliest role aggregates within its own output budget.
            // This deliberately gives a small max_characters budget a simple
            // stable meaning instead of replaying every role/bucket event.
            let mut roles = self
                .roles
                .iter()
                .filter_map(|(&char_id, role)| {
                    let aggregate = role.tree.aggregate();
                    (aggregate.hits > 0).then_some((aggregate.first_sequence, char_id, role))
                })
                .collect::<Vec<_>>();
            roles.sort_by_key(|(first_sequence, char_id, _)| (*first_sequence, *char_id));
            roles.truncate(character_budget);

            // One scratch buffer is reused for every role. Only touched slots
            // are read/reset, so 256 sparse roles over a 10k bucket chart do
            // not allocate and clear 2.56 million aggregates every refresh.
            let mut role_scratch = vec![CompactTimelineAggregate::default(); layout.bucket_count];
            #[cfg(test)]
            COMPACT_TIMELINE_ROLE_SCRATCH_SLOTS.with(|slots| {
                slots.set(slots.get().saturating_add(role_scratch.len()));
            });
            let mut touched_buckets = Vec::<usize>::new();
            let role_budget = max_roles_per_bucket.max(1).min(roles.len());
            let mut candidates = std::iter::repeat_with(|| None)
                .take(layout.bucket_count)
                .collect::<Vec<Option<CompactTimelineRoleCandidates>>>();
            for (role_index, (_, char_id, role)) in roles.iter().copied().enumerate() {
                role.tree.distribute(
                    layout,
                    start,
                    end,
                    &mut role_scratch,
                    Some(&mut touched_buckets),
                );
                for bucket_index in touched_buckets.drain(..) {
                    let aggregate = std::mem::take(&mut role_scratch[bucket_index]);
                    let bucket_candidates = candidates[bucket_index].get_or_insert_with(|| {
                        #[cfg(test)]
                        COMPACT_TIMELINE_ROLE_CANDIDATE_SLOTS.with(|slots| {
                            slots.set(slots.get().saturating_add(role_budget));
                        });
                        CompactTimelineRoleCandidates::new(role_budget)
                    });
                    bucket_candidates.consider(CompactTimelineRoleEvent {
                        first_sequence: aggregate.first_sequence,
                        char_id,
                        role_index,
                        aggregate,
                    });
                }
            }

            // Candidate sets are already independently bounded; only their
            // retained slots are materialized into the output. There is no
            // role_count * bucket_count event vector and no global event sort.
            for (bucket_index, candidates) in candidates.into_iter().enumerate() {
                let Some(candidates) = candidates else {
                    continue;
                };
                let candidate_len = candidates.len;
                for event in candidates
                    .slots
                    .into_vec()
                    .into_iter()
                    .take(candidate_len)
                    .flatten()
                {
                    let role = roles[event.role_index].2;
                    buckets[bucket_index].roles.push(CompactTimelineRoleBucket {
                        char_id: event.char_id,
                        char_name: role.name.clone(),
                        damage: event.aggregate.damage,
                        hits: event.aggregate.hits,
                    });
                }
            }
        }

        let mut omitted_role_damage = 0.0;
        let mut omitted_role_hits = 0_u64;
        for bucket in &mut buckets {
            let included_damage = bucket.roles.iter().map(|role| role.damage).sum::<f64>();
            let included_hits = bucket
                .roles
                .iter()
                .fold(0_u64, |total, role| total.saturating_add(role.hits));
            omitted_role_damage += (bucket.damage - included_damage).max(0.0);
            omitted_role_hits =
                omitted_role_hits.saturating_add(bucket.hits.saturating_sub(included_hits));
            bucket.roles.sort_by(|left, right| {
                right
                    .damage
                    .total_cmp(&left.damage)
                    .then_with(|| left.char_name.cmp(&right.char_name))
                    .then_with(|| left.char_id.cmp(&right.char_id))
            });
        }

        CompactTimelineSeries {
            bucket_seconds: layout.bucket_seconds,
            start_timestamp: Some(start),
            end_timestamp: Some(end),
            duration_seconds: buckets.last().map_or(0.0, |bucket| bucket.end_offset),
            omitted_role_damage,
            omitted_role_hits,
            buckets,
        }
    }
}

fn compact_timeline_ordered_key(timestamp: f64) -> u64 {
    let bits = timestamp.to_bits();
    if bits & (1_u64 << 63) == 0 {
        bits ^ (1_u64 << 63)
    } else {
        !bits
    }
}

fn compact_timeline_key_bit(key: u64, bit: u8) -> bool {
    key & (1_u64 << bit) != 0
}

/// A server HP decrease that calibration could observe but could not assign to
/// exactly one decoded hit. This is diagnostic evidence only: applying it does
/// not change team totals or any character, skill, or timeline attribution.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnattributedServerDamage {
    pub timestamp: f64,
    pub damage: f64,
    pub candidate_hits: u32,
}

/// Default source-side bound for timeline projections that do not expose a
/// caller-selected budget (for example legacy tests and HUD compatibility
/// helpers). Read-model APIs should call `timeline_bounded` with their own
/// contract limit. The authoritative hit history is never truncated.
pub const DEFAULT_MAX_TIMELINE_BUCKETS: usize = 20_000;
pub const DEFAULT_MAX_TIMELINE_ROLES_PER_BUCKET: usize = 256;
pub const DEFAULT_MAX_TIMELINE_CHARACTERS: usize = 256;
pub const MAX_TIMELINE_MARKERS: usize = 64;

#[derive(Clone, Copy, Debug)]
struct TimelineAggregationOptions {
    bucket_seconds: f64,
    subtract_time_stop: bool,
    max_buckets: usize,
    max_roles_per_bucket: usize,
    max_characters: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkillBreakdown {
    pub total_damage: f64,
    pub total_hits: u64,
    pub rows: Vec<SkillBreakdownRow>,
    pub unknown: UnknownAttributionSummary,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkillBreakdownRow {
    pub char_id: u32,
    pub char_name: String,
    pub name: String,
    pub category: String,
    pub ability_name: Option<String>,
    pub damage_name: Option<String>,
    pub gameplay_effect_index: Option<u32>,
    pub gameplay_effect_name: Option<String>,
    pub is_follow_up: bool,
    pub hits: u64,
    pub damage: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnknownAttributionSummary {
    pub unknown_character_count: usize,
    pub unknown_character_hits: u64,
    pub unknown_direction_hits: u64,
    pub unknown_direction_damage: f64,
    pub unmapped_skill_rows: usize,
    pub unmapped_skill_hits: u64,
    pub unmapped_skill_damage: f64,
    pub unmapped_gameplay_effects: Vec<UnknownGameplayEffect>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnknownGameplayEffect {
    pub index: u32,
    pub hits: u64,
    pub damage: f64,
}

/// The desktop skills contract accepts at most 4,096 rows. Reserve its last
/// row for a lossless damage/hit overflow aggregate so hostile or very long
/// replays cannot grow a second unbounded string-rich read model alongside the
/// authoritative hit log.
pub const MAX_INDEXED_SKILL_ROWS: usize = 4_095;
pub const MAX_INDEXED_UNKNOWN_CHARACTERS: usize = 63;
pub const MAX_INDEXED_UNMAPPED_GAMEPLAY_EFFECTS: usize = 511;
/// Six independently escaped text fields can appear on each of 4,096 rows.
/// An 80-byte ceiling keeps even worst-case JSON control-character escaping,
/// row metadata, and diagnostics below the 16 MiB stream delivery budget.
pub const MAX_INDEXED_SKILL_TEXT_BYTES: usize = 80;

const SKILL_OVERFLOW_CHARACTER_ID: u32 = u32::MAX;
const SKILL_OVERFLOW_EFFECT_INDEX: u32 = u32::MAX;

/// Incrementally maintained, bounded source projection shared by the skills
/// channel and capture diagnostics. The authoritative `hits` deque remains
/// complete; only this derived read model coalesces excess distinct keys.
#[derive(Clone, Debug, Default)]
struct SkillBreakdownIndex {
    rows: HashMap<SkillBreakdownKey, SkillBreakdownRow>,
    overflow_row: Option<SkillBreakdownRow>,
    overflow_unmapped_hits: u64,
    total_damage: f64,
    total_hits: u64,
    directions: HitDirectionSummary,
    unknown_characters: HashMap<u32, u64>,
    unknown_character_overflow_hits: u64,
    unknown_character_hits: u64,
    unmapped_skill_hits: u64,
    unmapped_skill_damage: f64,
    unmapped_gameplay_effects: HashMap<u32, UnknownGameplayEffect>,
    overflow_unmapped_gameplay_effect: Option<UnknownGameplayEffect>,
}

impl SkillBreakdownIndex {
    fn observe_hit(&mut self, hit: &Hit) {
        self.observe_direction(hit);
        if hit.direction.is_incoming() {
            return;
        }

        if !hit.char_known {
            self.observe_unknown_character(hit.char_id);
            self.unknown_character_hits = self.unknown_character_hits.saturating_add(1);
        }

        if hit.damage.is_finite() && hit.damage > 0.0 {
            self.observe_entry(hit, SkillEntryRef::from_hit(hit, false), hit.damage);
            self.total_damage += hit.damage;
            self.total_hits = self.total_hits.saturating_add(1);
            if is_hit_skill_unmapped(hit) {
                self.unmapped_skill_hits = self.unmapped_skill_hits.saturating_add(1);
                self.unmapped_skill_damage += hit.damage;
            }
            if let Some(index) = hit.gameplay_effect_index
                && hit.gameplay_effect_name.is_none()
            {
                self.observe_unmapped_effect(index, hit.damage);
            }
        }
        if hit.follow_up_damage.is_finite() && hit.follow_up_damage > 0.0 {
            self.observe_entry(
                hit,
                SkillEntryRef::from_hit(hit, true),
                hit.follow_up_damage,
            );
            self.total_damage += hit.follow_up_damage;
            self.total_hits = self.total_hits.saturating_add(1);
        }
    }

    fn remove_hit(&mut self, hit: &Hit) {
        self.remove_direction(hit);
        if hit.direction.is_incoming() {
            return;
        }

        if !hit.char_known {
            self.remove_unknown_character(hit.char_id);
            self.unknown_character_hits = self.unknown_character_hits.saturating_sub(1);
        }

        if hit.damage.is_finite() && hit.damage > 0.0 {
            self.remove_entry(hit, SkillEntryRef::from_hit(hit, false), hit.damage);
            self.total_damage = (self.total_damage - hit.damage).max(0.0);
            self.total_hits = self.total_hits.saturating_sub(1);
            if is_hit_skill_unmapped(hit) {
                self.unmapped_skill_hits = self.unmapped_skill_hits.saturating_sub(1);
                self.unmapped_skill_damage = (self.unmapped_skill_damage - hit.damage).max(0.0);
            }
            if let Some(index) = hit.gameplay_effect_index
                && hit.gameplay_effect_name.is_none()
            {
                self.remove_unmapped_effect(index, hit.damage);
            }
        }
        if hit.follow_up_damage.is_finite() && hit.follow_up_damage > 0.0 {
            self.remove_entry(
                hit,
                SkillEntryRef::from_hit(hit, true),
                hit.follow_up_damage,
            );
            self.total_damage = (self.total_damage - hit.follow_up_damage).max(0.0);
            self.total_hits = self.total_hits.saturating_sub(1);
        }
    }

    fn replace_hit(&mut self, before: &Hit, after: &Hit) {
        self.remove_hit(before);
        self.observe_hit(after);
    }

    fn promote_unknown_direction(&mut self, damage: f64) {
        self.directions.unknown_hits = self.directions.unknown_hits.saturating_sub(1);
        self.directions.unknown_damage = (self.directions.unknown_damage - damage).max(0.0);
        self.directions.outgoing_hits = self.directions.outgoing_hits.saturating_add(1);
        self.directions.outgoing_damage += damage;
    }

    fn observe_direction(&mut self, hit: &Hit) {
        let damage = hit.total_damage();
        match hit.direction {
            HitDirection::Outgoing => {
                self.directions.outgoing_hits = self.directions.outgoing_hits.saturating_add(1);
                self.directions.outgoing_damage += damage;
            }
            HitDirection::Unknown => {
                self.directions.unknown_hits = self.directions.unknown_hits.saturating_add(1);
                self.directions.unknown_damage += damage;
            }
            HitDirection::Incoming => {
                self.directions.incoming_hits = self.directions.incoming_hits.saturating_add(1);
                self.directions.incoming_damage += damage;
            }
        }
    }

    fn remove_direction(&mut self, hit: &Hit) {
        let damage = hit.total_damage();
        match hit.direction {
            HitDirection::Outgoing => {
                self.directions.outgoing_hits = self.directions.outgoing_hits.saturating_sub(1);
                self.directions.outgoing_damage =
                    (self.directions.outgoing_damage - damage).max(0.0);
            }
            HitDirection::Unknown => {
                self.directions.unknown_hits = self.directions.unknown_hits.saturating_sub(1);
                self.directions.unknown_damage = (self.directions.unknown_damage - damage).max(0.0);
            }
            HitDirection::Incoming => {
                self.directions.incoming_hits = self.directions.incoming_hits.saturating_sub(1);
                self.directions.incoming_damage =
                    (self.directions.incoming_damage - damage).max(0.0);
            }
        }
    }

    fn observe_unknown_character(&mut self, char_id: u32) {
        if let Some(hits) = self.unknown_characters.get_mut(&char_id) {
            *hits = hits.saturating_add(1);
        } else if self.unknown_characters.len() < MAX_INDEXED_UNKNOWN_CHARACTERS {
            self.unknown_characters.insert(char_id, 1);
        } else {
            self.unknown_character_overflow_hits =
                self.unknown_character_overflow_hits.saturating_add(1);
        }
    }

    fn remove_unknown_character(&mut self, char_id: u32) {
        let remove = match self.unknown_characters.get_mut(&char_id) {
            Some(hits) => {
                *hits = hits.saturating_sub(1);
                *hits == 0
            }
            None => {
                self.unknown_character_overflow_hits =
                    self.unknown_character_overflow_hits.saturating_sub(1);
                false
            }
        };
        if remove {
            self.unknown_characters.remove(&char_id);
        }
    }

    fn observe_entry(&mut self, hit: &Hit, entry: SkillEntryRef, damage: f64) {
        let key = skill_breakdown_key(hit, &entry);
        if self.rows.contains_key(&key) || self.rows.len() < MAX_INDEXED_SKILL_ROWS {
            push_skill_breakdown_entry(&mut self.rows, hit, entry, damage);
            return;
        }
        let row = self.overflow_row.get_or_insert_with(|| SkillBreakdownRow {
            char_id: SKILL_OVERFLOW_CHARACTER_ID,
            char_name: "Shared".to_owned(),
            name: "Aggregated Overflow".to_owned(),
            category: "Other".to_owned(),
            ability_name: None,
            damage_name: None,
            gameplay_effect_index: None,
            gameplay_effect_name: None,
            is_follow_up: false,
            hits: 0,
            damage: 0.0,
        });
        row.hits = row.hits.saturating_add(1);
        row.damage += damage;
        if !entry.is_follow_up
            && entry.damage_name.is_none()
            && entry.ability_name.is_none()
            && entry.gameplay_effect_name.is_none()
        {
            self.overflow_unmapped_hits = self.overflow_unmapped_hits.saturating_add(1);
        }
    }

    fn remove_entry(&mut self, hit: &Hit, entry: SkillEntryRef, damage: f64) {
        let key = skill_breakdown_key(hit, &entry);
        let mut remove_key = false;
        if let Some(row) = self.rows.get_mut(&key) {
            row.hits = row.hits.saturating_sub(1);
            row.damage = (row.damage - damage).max(0.0);
            remove_key = row.hits == 0;
        } else if let Some(row) = self.overflow_row.as_mut() {
            row.hits = row.hits.saturating_sub(1);
            row.damage = (row.damage - damage).max(0.0);
            if !entry.is_follow_up
                && entry.damage_name.is_none()
                && entry.ability_name.is_none()
                && entry.gameplay_effect_name.is_none()
            {
                self.overflow_unmapped_hits = self.overflow_unmapped_hits.saturating_sub(1);
            }
            if row.hits == 0 {
                self.overflow_row = None;
                self.overflow_unmapped_hits = 0;
            }
        }
        if remove_key {
            self.rows.remove(&key);
        }
    }

    fn observe_unmapped_effect(&mut self, index: u32, damage: f64) {
        if index == SKILL_OVERFLOW_EFFECT_INDEX {
            let effect =
                self.overflow_unmapped_gameplay_effect
                    .get_or_insert(UnknownGameplayEffect {
                        index: SKILL_OVERFLOW_EFFECT_INDEX,
                        hits: 0,
                        damage: 0.0,
                    });
            effect.hits = effect.hits.saturating_add(1);
            effect.damage += damage;
            return;
        }
        if let Some(effect) = self.unmapped_gameplay_effects.get_mut(&index) {
            effect.hits = effect.hits.saturating_add(1);
            effect.damage += damage;
        } else if self.unmapped_gameplay_effects.len() < MAX_INDEXED_UNMAPPED_GAMEPLAY_EFFECTS {
            self.unmapped_gameplay_effects.insert(
                index,
                UnknownGameplayEffect {
                    index,
                    hits: 1,
                    damage,
                },
            );
        } else {
            let effect =
                self.overflow_unmapped_gameplay_effect
                    .get_or_insert(UnknownGameplayEffect {
                        index: SKILL_OVERFLOW_EFFECT_INDEX,
                        hits: 0,
                        damage: 0.0,
                    });
            effect.hits = effect.hits.saturating_add(1);
            effect.damage += damage;
        }
    }

    fn remove_unmapped_effect(&mut self, index: u32, damage: f64) {
        if index == SKILL_OVERFLOW_EFFECT_INDEX {
            if let Some(effect) = self.overflow_unmapped_gameplay_effect.as_mut() {
                effect.hits = effect.hits.saturating_sub(1);
                effect.damage = (effect.damage - damage).max(0.0);
                if effect.hits == 0 {
                    self.overflow_unmapped_gameplay_effect = None;
                }
            }
            return;
        }
        let remove = match self.unmapped_gameplay_effects.get_mut(&index) {
            Some(effect) => {
                effect.hits = effect.hits.saturating_sub(1);
                effect.damage = (effect.damage - damage).max(0.0);
                effect.hits == 0
            }
            None => {
                if let Some(effect) = self.overflow_unmapped_gameplay_effect.as_mut() {
                    effect.hits = effect.hits.saturating_sub(1);
                    effect.damage = (effect.damage - damage).max(0.0);
                    if effect.hits == 0 {
                        self.overflow_unmapped_gameplay_effect = None;
                    }
                }
                false
            }
        };
        if remove {
            self.unmapped_gameplay_effects.remove(&index);
        }
    }

    fn project(&self) -> SkillBreakdown {
        let mut rows = self.rows.values().cloned().collect::<Vec<_>>();
        if let Some(overflow) = self.overflow_row.clone() {
            rows.push(overflow);
        }
        #[cfg(test)]
        SKILL_INDEX_PROJECTION_VISITS.with(|visits| {
            visits.set(visits.get().saturating_add(rows.len()));
        });
        rows.sort_by(|left, right| {
            right
                .damage
                .total_cmp(&left.damage)
                .then_with(|| left.char_name.cmp(&right.char_name))
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.is_follow_up.cmp(&right.is_follow_up))
        });
        let mut effects = self
            .unmapped_gameplay_effects
            .values()
            .cloned()
            .collect::<Vec<_>>();
        if let Some(overflow) = self.overflow_unmapped_gameplay_effect.clone() {
            effects.push(overflow);
        }
        effects.sort_by_key(|effect| effect.index);
        let unknown_character_count =
            self.unknown_characters.len() + usize::from(self.unknown_character_overflow_hits > 0);
        SkillBreakdown {
            total_damage: self.total_damage,
            total_hits: self.total_hits,
            unknown: UnknownAttributionSummary {
                unknown_character_count,
                unknown_character_hits: self.unknown_character_hits,
                unknown_direction_hits: self.directions.unknown_hits,
                unknown_direction_damage: self.directions.unknown_damage,
                unmapped_skill_rows: self
                    .rows
                    .values()
                    .filter(|row| is_unmapped_skill_row(row))
                    .count()
                    + usize::from(self.overflow_unmapped_hits > 0),
                unmapped_skill_hits: self.unmapped_skill_hits,
                unmapped_skill_damage: self.unmapped_skill_damage,
                unmapped_gameplay_effects: effects,
            },
            rows,
        }
    }

    fn unknown_character_count(&self) -> usize {
        self.unknown_characters.len() + usize::from(self.unknown_character_overflow_hits > 0)
    }

    fn unmapped_skill_row_count(&self) -> usize {
        self.rows
            .values()
            .filter(|row| is_unmapped_skill_row(row))
            .count()
            + usize::from(self.overflow_unmapped_hits > 0)
    }

    fn unmapped_gameplay_effect_count(&self) -> usize {
        self.unmapped_gameplay_effects.len()
            + usize::from(self.overflow_unmapped_gameplay_effect.is_some())
    }
}

#[cfg(test)]
thread_local! {
    static SKILL_INDEX_PROJECTION_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn reset_skill_index_projection_visits() {
    SKILL_INDEX_PROJECTION_VISITS.with(|visits| visits.set(0));
}

#[cfg(test)]
fn skill_index_projection_visits() -> usize {
    SKILL_INDEX_PROJECTION_VISITS.with(std::cell::Cell::get)
}

const MAX_INDEXED_DETAIL_CHARACTERS: usize = 256;
const MAX_INDEXED_DETAIL_SKILLS: usize = 4_095;
const MAX_INDEXED_DETAIL_QTE_TYPES: usize = 511;
pub const MAX_INDEXED_DETAIL_KEY_BYTES: usize = 256;
const MAX_INDEXED_DETAIL_AGGREGATES: usize = 65_536;
const DETAIL_OVERFLOW_SKILL: &str = "__nte_aggregated_other_skills__";
const DETAIL_OVERFLOW_QTE: &str = "__nte_aggregated_other_qte__";

/// Stable filter key used by the reducer-maintained hit-detail index. Keeping
/// this frontend-neutral lets Tauri request a page without walking all hits.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum IndexedCombatDetailFilter {
    All,
    Outgoing,
    Incoming,
    CharacterAttributed,
    CharacterDirect,
    ReactionDamage,
    SharedMechanics,
    Unattributed,
    QteType(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IndexedDetailQteSummary {
    pub attack_type: String,
    pub hits: u64,
    pub damage: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IndexedDetailSkillSummary {
    pub id: String,
    pub representative_position: usize,
    pub hits: u64,
    pub damage: f64,
}

pub struct IndexedCombatDetailPage<'a> {
    pub total_hits: usize,
    pub total_damage: f64,
    pub max_row_damage: f64,
    pub rows: Vec<(usize, &'a Hit)>,
    pub directions: HitDirectionSummary,
    pub qte_summaries: Vec<IndexedDetailQteSummary>,
    pub skill_summaries: Vec<IndexedDetailSkillSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct IndexedDetailBucketKey {
    character_id: Option<u32>,
    filter: IndexedCombatDetailFilter,
    skill: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct IndexedDetailBucket {
    hits: usize,
    total_damage: f64,
    stable_max_damage: f64,
}

impl IndexedDetailBucket {
    fn insert(&mut self, damage: f64) {
        self.hits = self.hits.saturating_add(1);
        self.total_damage += damage;
    }

    fn remove(&mut self, damage: f64) {
        self.hits = self.hits.saturating_sub(1);
        self.total_damage = (self.total_damage - damage).max(0.0);
    }
}

#[derive(Clone, Debug, Default)]
struct IndexedDetailPositions(Vec<usize>);

impl IndexedDetailPositions {
    fn insert(&mut self, position: usize) {
        if self.0.last().is_none_or(|last| *last < position) {
            self.0.push(position);
            return;
        }
        if let Err(index) = self.0.binary_search(&position) {
            self.0.insert(index, position);
        }
    }

    fn remove(&mut self, position: usize) {
        if let Ok(index) = self.0.binary_search(&position) {
            self.0.remove(index);
        }
    }
}

#[derive(Clone, Debug, Default)]
struct IndexedDetailAggregate {
    hits: u64,
    damage: f64,
    representative_position: usize,
}

#[derive(Clone, Debug, Default)]
struct IndexedDetailScopeSummary {
    directions: HitDirectionSummary,
    qte: HashMap<String, IndexedDetailAggregate>,
    qte_overflow: IndexedDetailAggregate,
    skills: HashMap<String, IndexedDetailAggregate>,
    skill_overflow: IndexedDetailAggregate,
}

#[derive(Clone, Debug, Default)]
struct CombatDetailIndex {
    buckets: HashMap<IndexedDetailBucketKey, IndexedDetailBucket>,
    scopes: HashMap<Option<u32>, IndexedDetailScopeSummary>,
    characters: HashMap<u32, u64>,
    skills: HashMap<String, u64>,
    qte_types: HashMap<String, u64>,
    character_positions: HashMap<u32, IndexedDetailPositions>,
    skill_positions: HashMap<String, IndexedDetailPositions>,
    filter_positions: HashMap<IndexedCombatDetailFilter, IndexedDetailPositions>,
    scope_aggregate_count: usize,
    scope_aggregates_saturated: bool,
}

impl CombatDetailIndex {
    fn observe_hit(&mut self, position: usize, hit: &Hit) {
        let force_overflow = self.buckets.len().saturating_add(32) >= MAX_INDEXED_DETAIL_AGGREGATES;
        let character_id = self.observe_character(hit.char_id, force_overflow);
        let skill = self.observe_skill(hit_skill_id_ref(hit), force_overflow);
        let qte_types = self.observe_qte_types(hit, force_overflow);
        let filters = indexed_detail_filters_for_hit(hit, &qte_types);
        let damage = hit.total_damage();
        self.character_positions
            .entry(character_id)
            .or_default()
            .insert(position);
        self.skill_positions
            .entry(skill.clone())
            .or_default()
            .insert(position);
        for filter in filters
            .iter()
            .filter(|filter| !matches!(filter, IndexedCombatDetailFilter::All))
        {
            self.filter_positions
                .entry(filter.clone())
                .or_default()
                .insert(position);
        }
        for scope in [None, Some(character_id)] {
            for filter in &filters {
                for skill_filter in [None, Some(skill.clone())] {
                    let key = IndexedDetailBucketKey {
                        character_id: scope,
                        filter: filter.clone(),
                        skill: skill_filter,
                    };
                    if self.buckets.contains_key(&key)
                        || self.buckets.len() < MAX_INDEXED_DETAIL_AGGREGATES
                    {
                        self.buckets.entry(key).or_default().insert(damage);
                    }
                }
            }
            self.observe_scope(scope, position, hit, &skill, &qte_types);
        }
    }

    fn remove_hit(&mut self, position: usize, hit: &Hit) {
        let character_id = self.normalized_character(hit.char_id);
        let skill = self.normalized_skill(hit_skill_id_ref(hit)).to_owned();
        let qte_types = normalized_qte_types(self, hit);
        let filters = indexed_detail_filters_for_hit(hit, &qte_types);
        let damage = hit.total_damage();
        if let Some(index) = self.character_positions.get_mut(&character_id) {
            index.remove(position);
        }
        if let Some(index) = self.skill_positions.get_mut(&skill) {
            index.remove(position);
        }
        for filter in filters
            .iter()
            .filter(|filter| !matches!(filter, IndexedCombatDetailFilter::All))
        {
            if let Some(index) = self.filter_positions.get_mut(filter) {
                index.remove(position);
            }
        }
        for scope in [None, Some(character_id)] {
            for filter in &filters {
                for skill_filter in [None, Some(skill.clone())] {
                    let key = IndexedDetailBucketKey {
                        character_id: scope,
                        filter: filter.clone(),
                        skill: skill_filter,
                    };
                    let remove_bucket = if let Some(bucket) = self.buckets.get_mut(&key) {
                        bucket.remove(damage);
                        bucket.hits == 0
                    } else {
                        false
                    };
                    if remove_bucket {
                        self.buckets.remove(&key);
                    }
                }
            }
            self.remove_scope(scope, position, hit, &skill, &qte_types);
        }
        decrement_bounded_key(&mut self.characters, &hit.char_id);
        decrement_bounded_key(&mut self.skills, hit_skill_id_ref(hit));
        for qte in raw_qte_types(hit) {
            decrement_bounded_key(&mut self.qte_types, qte);
        }
    }

    fn replace_hit(&mut self, position: usize, before: &Hit, after: &Hit) {
        self.remove_hit(position, before);
        self.observe_hit(position, after);
    }

    fn promote_stable_hit(&mut self, hit: &Hit) {
        let character_id = self.normalized_character(hit.char_id);
        let skill = self.normalized_skill(hit_skill_id_ref(hit)).to_owned();
        let filters = indexed_detail_filters_for_hit(hit, &normalized_qte_types(self, hit));
        for scope in [None, Some(character_id)] {
            for filter in &filters {
                for skill_filter in [None, Some(skill.clone())] {
                    if let Some(bucket) = self.buckets.get_mut(&IndexedDetailBucketKey {
                        character_id: scope,
                        filter: filter.clone(),
                        skill: skill_filter,
                    }) {
                        bucket.stable_max_damage = bucket.stable_max_damage.max(hit.total_damage());
                    }
                }
            }
        }
    }

    fn query<'a>(
        &'a self,
        hits: &'a VecDeque<Hit>,
        character_id: Option<u32>,
        filter: &IndexedCombatDetailFilter,
        skill: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> IndexedCombatDetailPage<'a> {
        let scope = character_id.map(|value| self.normalized_character(value));
        let filter = match filter {
            IndexedCombatDetailFilter::QteType(value) => {
                IndexedCombatDetailFilter::QteType(self.normalized_qte(value).to_owned())
            }
            value => value.clone(),
        };
        let skill = skill.map(|value| self.normalized_skill(value).to_owned());
        let key = IndexedDetailBucketKey {
            character_id: scope,
            filter: filter.clone(),
            skill: skill.clone(),
        };
        let bucket = self.buckets.get(&key);
        let total_hits = bucket.map_or(0, |bucket| bucket.hits);
        let mut candidates = scope
            .and_then(|value| self.character_positions.get(&value))
            .map(|value| value.0.as_slice());
        if !matches!(filter, IndexedCombatDetailFilter::All) {
            candidates = choose_shorter_detail_positions(
                candidates,
                Some(
                    self.filter_positions
                        .get(&filter)
                        .map_or(&[], |value| value.0.as_slice()),
                ),
            );
        }
        if let Some(skill) = skill.as_deref() {
            candidates = choose_shorter_detail_positions(
                candidates,
                Some(
                    self.skill_positions
                        .get(skill)
                        .map_or(&[], |value| value.0.as_slice()),
                ),
            );
        }
        let constraint_count = usize::from(scope.is_some())
            + usize::from(!matches!(filter, IndexedCombatDetailFilter::All))
            + usize::from(skill.is_some());
        let page_end = offset.saturating_add(limit).min(total_hits);
        let rows = match (constraint_count, candidates) {
            (0, _) => collect_exact_indexed_detail_page(hits, offset..page_end),
            (1, Some(candidates)) => collect_exact_indexed_detail_page(
                hits,
                candidates
                    .get(offset..page_end)
                    .unwrap_or_default()
                    .iter()
                    .copied(),
            ),
            (_, Some(candidates)) => collect_indexed_detail_page(
                self,
                hits,
                candidates.iter().copied(),
                scope,
                &filter,
                skill.as_deref(),
                total_hits,
                offset,
                limit,
            ),
            (_, None) => collect_indexed_detail_page(
                self,
                hits,
                0..hits.len(),
                scope,
                &filter,
                skill.as_deref(),
                total_hits,
                offset,
                limit,
            ),
        };
        let recent_max = hits
            .iter()
            .skip(hits.len().saturating_sub(RECENT_HIT_MUTATION_WINDOW))
            .filter(|hit| self.matches_query(hit, scope, &filter, skill.as_deref()))
            .map(Hit::total_damage)
            .fold(1.0_f64, f64::max);
        let summary = self.scopes.get(&scope);
        let mut qte_summaries = summary
            .into_iter()
            .flat_map(|summary| &summary.qte)
            .map(|(attack_type, row)| IndexedDetailQteSummary {
                attack_type: attack_type.clone(),
                hits: row.hits,
                damage: row.damage,
            })
            .collect::<Vec<_>>();
        if let Some(summary) = summary
            && summary.qte_overflow.hits > 0
        {
            qte_summaries.push(IndexedDetailQteSummary {
                attack_type: DETAIL_OVERFLOW_QTE.to_owned(),
                hits: summary.qte_overflow.hits,
                damage: summary.qte_overflow.damage,
            });
        }
        qte_summaries.sort_by(|left, right| right.damage.total_cmp(&left.damage));
        let mut skill_summaries = summary
            .into_iter()
            .flat_map(|summary| &summary.skills)
            .map(|(id, row)| IndexedDetailSkillSummary {
                id: id.clone(),
                representative_position: row.representative_position,
                hits: row.hits,
                damage: row.damage,
            })
            .collect::<Vec<_>>();
        if let Some(summary) = summary
            && summary.skill_overflow.hits > 0
        {
            skill_summaries.push(IndexedDetailSkillSummary {
                id: DETAIL_OVERFLOW_SKILL.to_owned(),
                representative_position: summary.skill_overflow.representative_position,
                hits: summary.skill_overflow.hits,
                damage: summary.skill_overflow.damage,
            });
        }
        skill_summaries.sort_by(|left, right| right.damage.total_cmp(&left.damage));
        IndexedCombatDetailPage {
            total_hits,
            total_damage: bucket.map_or(0.0, |bucket| bucket.total_damage),
            max_row_damage: bucket.map_or(1.0, |bucket| {
                bucket.stable_max_damage.max(recent_max).max(1.0)
            }),
            rows,
            directions: summary.map_or(HitDirectionSummary::default(), |row| row.directions),
            qte_summaries,
            skill_summaries,
        }
    }

    fn observe_character(&mut self, char_id: u32, force_overflow: bool) -> u32 {
        if self.characters.contains_key(&char_id)
            || (!force_overflow && self.characters.len() < MAX_INDEXED_DETAIL_CHARACTERS)
        {
            *self.characters.entry(char_id).or_default() += 1;
            char_id
        } else {
            SKILL_OVERFLOW_CHARACTER_ID
        }
    }

    fn normalized_character(&self, char_id: u32) -> u32 {
        if self.characters.contains_key(&char_id) {
            char_id
        } else {
            SKILL_OVERFLOW_CHARACTER_ID
        }
    }

    fn observe_skill(&mut self, skill: &str, force_overflow: bool) -> String {
        let skill = bounded_detail_key(skill, DETAIL_OVERFLOW_SKILL);
        if self.skills.contains_key(&skill)
            || (!force_overflow && self.skills.len() < MAX_INDEXED_DETAIL_SKILLS)
        {
            *self.skills.entry(skill.clone()).or_default() += 1;
            skill
        } else {
            DETAIL_OVERFLOW_SKILL.to_owned()
        }
    }

    fn normalized_skill<'a>(&'a self, skill: &'a str) -> &'a str {
        if skill.len() <= MAX_INDEXED_DETAIL_KEY_BYTES && self.skills.contains_key(skill) {
            skill
        } else {
            DETAIL_OVERFLOW_SKILL
        }
    }

    fn observe_qte_types(&mut self, hit: &Hit, force_overflow: bool) -> Vec<String> {
        raw_qte_types(hit)
            .into_iter()
            .map(|qte| {
                let qte = bounded_detail_key(qte, DETAIL_OVERFLOW_QTE);
                if self.qte_types.contains_key(&qte)
                    || (!force_overflow && self.qte_types.len() < MAX_INDEXED_DETAIL_QTE_TYPES)
                {
                    *self.qte_types.entry(qte.clone()).or_default() += 1;
                    qte
                } else {
                    DETAIL_OVERFLOW_QTE.to_owned()
                }
            })
            .collect()
    }

    fn normalized_qte<'a>(&'a self, qte: &'a str) -> &'a str {
        if qte.len() <= MAX_INDEXED_DETAIL_KEY_BYTES && self.qte_types.contains_key(qte) {
            qte
        } else {
            DETAIL_OVERFLOW_QTE
        }
    }

    fn observe_scope(
        &mut self,
        scope: Option<u32>,
        position: usize,
        hit: &Hit,
        skill: &str,
        qte_types: &[String],
    ) {
        let summary = self.scopes.entry(scope).or_default();
        add_hit_direction(&mut summary.directions, hit);
        if hit.direction.is_incoming() {
            return;
        }
        let skill_row = if let Some(row) = summary.skills.get_mut(skill) {
            row
        } else if !self.scope_aggregates_saturated
            && self.scope_aggregate_count < MAX_INDEXED_DETAIL_AGGREGATES
        {
            self.scope_aggregate_count = self.scope_aggregate_count.saturating_add(1);
            summary.skills.entry(skill.to_owned()).or_default()
        } else {
            self.scope_aggregates_saturated = true;
            &mut summary.skill_overflow
        };
        if skill_row.hits == 0 {
            skill_row.representative_position = position;
        }
        skill_row.hits = skill_row.hits.saturating_add(1);
        skill_row.damage += hit.total_damage();
        for qte in qte_types {
            let (hits, damage) = qte_detail_contribution(hit, qte);
            let row = if let Some(row) = summary.qte.get_mut(qte) {
                row
            } else if !self.scope_aggregates_saturated
                && self.scope_aggregate_count < MAX_INDEXED_DETAIL_AGGREGATES
            {
                self.scope_aggregate_count = self.scope_aggregate_count.saturating_add(1);
                summary.qte.entry(qte.clone()).or_default()
            } else {
                self.scope_aggregates_saturated = true;
                &mut summary.qte_overflow
            };
            if row.hits == 0 {
                row.representative_position = position;
            }
            row.hits = row.hits.saturating_add(hits);
            row.damage += damage;
        }
    }

    fn remove_scope(
        &mut self,
        scope: Option<u32>,
        _position: usize,
        hit: &Hit,
        skill: &str,
        qte_types: &[String],
    ) {
        let Some(summary) = self.scopes.get_mut(&scope) else {
            return;
        };
        remove_hit_direction(&mut summary.directions, hit);
        if !hit.direction.is_incoming() {
            if summary.skills.contains_key(skill) {
                if remove_detail_aggregate(&mut summary.skills, skill, hit.total_damage()) {
                    self.scope_aggregate_count = self.scope_aggregate_count.saturating_sub(1);
                }
            } else {
                remove_inline_detail_aggregate(&mut summary.skill_overflow, 1, hit.total_damage());
            }
            for qte in qte_types {
                let (hits, damage) = qte_detail_contribution(hit, qte);
                if summary.qte.contains_key(qte) {
                    if remove_detail_aggregate_counts(&mut summary.qte, qte, hits, damage) {
                        self.scope_aggregate_count = self.scope_aggregate_count.saturating_sub(1);
                    }
                } else {
                    remove_inline_detail_aggregate(&mut summary.qte_overflow, hits, damage);
                }
            }
        }
    }

    fn matches_query(
        &self,
        hit: &Hit,
        character_id: Option<u32>,
        filter: &IndexedCombatDetailFilter,
        skill: Option<&str>,
    ) -> bool {
        character_id.is_none_or(|value| self.normalized_character(hit.char_id) == value)
            && indexed_detail_filter_matches(filter, hit, self)
            && skill.is_none_or(|value| self.normalized_skill(hit_skill_id_ref(hit)) == value)
    }

    #[cfg(test)]
    fn position_slot_count(&self) -> usize {
        self.character_positions
            .values()
            .chain(self.skill_positions.values())
            .chain(self.filter_positions.values())
            .map(|positions| positions.0.len())
            .sum()
    }
}

fn collect_exact_indexed_detail_page(
    hits: &VecDeque<Hit>,
    positions: impl Iterator<Item = usize>,
) -> Vec<(usize, &Hit)> {
    positions
        .filter_map(|position| {
            let hit = hits.get(position)?;
            #[cfg(test)]
            COMBAT_DETAIL_QUERY_VISITS.with(|visits| {
                visits.set(visits.get().saturating_add(1));
            });
            Some((position, hit))
        })
        .collect()
}

fn bounded_detail_key(value: &str, fallback: &str) -> String {
    if value.len() <= MAX_INDEXED_DETAIL_KEY_BYTES {
        value.to_owned()
    } else {
        fallback.to_owned()
    }
}

fn choose_shorter_detail_positions<'a>(
    left: Option<&'a [usize]>,
    right: Option<&'a [usize]>,
) -> Option<&'a [usize]> {
    match (left, right) {
        (Some(left), Some(right)) => Some(if left.len() <= right.len() {
            left
        } else {
            right
        }),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_indexed_detail_page<'a>(
    index: &CombatDetailIndex,
    hits: &'a VecDeque<Hit>,
    positions: impl DoubleEndedIterator<Item = usize>,
    character_id: Option<u32>,
    filter: &IndexedCombatDetailFilter,
    skill: Option<&str>,
    total_hits: usize,
    offset: usize,
    limit: usize,
) -> Vec<(usize, &'a Hit)> {
    if limit == 0 || offset >= total_hits {
        return Vec::new();
    }
    let page_end = offset.saturating_add(limit).min(total_hits);
    let page_len = page_end.saturating_sub(offset);
    let forward_matches = page_end;
    let reverse_matches = total_hits.saturating_sub(offset);
    let mut rows = Vec::with_capacity(page_len);

    // The aggregate bucket already knows the exact match count. For a deep
    // page, walk the shortest retained suffix instead of replaying every
    // matching hit before `offset` while the authoritative state lock is held.
    // Candidate position lists are ordered, so reversing again below preserves
    // the stable chronological/page order.
    if reverse_matches < forward_matches {
        let mut skipped_after_page = 0usize;
        let skip_after_page = total_hits.saturating_sub(page_end);
        for position in positions.rev() {
            let Some(hit) = hits.get(position) else {
                continue;
            };
            #[cfg(test)]
            COMBAT_DETAIL_QUERY_VISITS.with(|visits| {
                visits.set(visits.get().saturating_add(1));
            });
            if !index.matches_query(hit, character_id, filter, skill) {
                continue;
            }
            if skipped_after_page < skip_after_page {
                skipped_after_page = skipped_after_page.saturating_add(1);
                continue;
            }
            if rows.len() < page_len {
                rows.push((position, hit));
                if rows.len() >= page_len {
                    break;
                }
            } else {
                break;
            }
        }
        rows.reverse();
        return rows;
    }

    let mut matched = 0usize;
    for position in positions {
        let Some(hit) = hits.get(position) else {
            continue;
        };
        #[cfg(test)]
        COMBAT_DETAIL_QUERY_VISITS.with(|visits| {
            visits.set(visits.get().saturating_add(1));
        });
        if !index.matches_query(hit, character_id, filter, skill) {
            continue;
        }
        if matched >= offset {
            rows.push((position, hit));
        }
        matched = matched.saturating_add(1);
        if matched >= page_end {
            break;
        }
    }
    rows
}

fn decrement_bounded_key<K, Q>(values: &mut HashMap<K, u64>, key: &Q)
where
    K: std::hash::Hash + Eq + std::borrow::Borrow<Q>,
    Q: std::hash::Hash + Eq + ?Sized,
{
    let remove = values.get_mut(key).is_some_and(|count| {
        *count = count.saturating_sub(1);
        *count == 0
    });
    if remove {
        values.remove(key);
    }
}

fn hit_skill_id_ref(hit: &Hit) -> &str {
    hit.damage_component
        .as_deref()
        .or(hit.ability_name.as_deref())
        .or(hit.gameplay_effect_name.as_deref())
        .or(hit.damage_name.as_deref())
        .or(hit.attack_type.as_deref())
        .unwrap_or("Unmapped Skill")
}

fn raw_qte_types(hit: &Hit) -> Vec<&str> {
    if hit.direction.is_incoming() {
        return Vec::new();
    }
    let mut values = Vec::with_capacity(2);
    if let Some(value) = hit.attack_type.as_deref()
        && (is_qte_follow_up_damage_type(value) || is_unbalance_damage_hit(hit))
    {
        values.push(value);
    }
    if hit.follow_up_damage > 0.0
        && let Some(value) = hit.follow_up_attack_type.as_deref()
        && is_qte_follow_up_damage_type(value)
        && !values.contains(&value)
    {
        values.push(value);
    }
    values
}

fn normalized_qte_types(index: &CombatDetailIndex, hit: &Hit) -> Vec<String> {
    raw_qte_types(hit)
        .into_iter()
        .map(|value| index.normalized_qte(value).to_owned())
        .collect()
}

fn indexed_detail_filters_for_hit(
    hit: &Hit,
    qte_types: &[String],
) -> Vec<IndexedCombatDetailFilter> {
    let mut filters = vec![IndexedCombatDetailFilter::All];
    if hit.direction.is_incoming() {
        filters.push(IndexedCombatDetailFilter::Incoming);
        return filters;
    }
    filters.push(IndexedCombatDetailFilter::Outgoing);
    let unbalance = is_unbalance_damage_hit(hit);
    if hit.direction.is_outgoing() && hit.char_known && !unbalance {
        filters.push(IndexedCombatDetailFilter::CharacterAttributed);
        let reaction = reaction_damage_for_hit(hit);
        if hit.total_damage() > reaction {
            filters.push(IndexedCombatDetailFilter::CharacterDirect);
        }
        if reaction > 0.0 {
            filters.push(IndexedCombatDetailFilter::ReactionDamage);
        }
    }
    if unbalance {
        filters.push(IndexedCombatDetailFilter::SharedMechanics);
    }
    if !(unbalance || (hit.direction.is_outgoing() && hit.char_known)) {
        filters.push(IndexedCombatDetailFilter::Unattributed);
    }
    filters.extend(
        qte_types
            .iter()
            .cloned()
            .map(IndexedCombatDetailFilter::QteType),
    );
    filters
}

fn indexed_detail_filter_matches(
    filter: &IndexedCombatDetailFilter,
    hit: &Hit,
    index: &CombatDetailIndex,
) -> bool {
    match filter {
        IndexedCombatDetailFilter::All => true,
        IndexedCombatDetailFilter::Outgoing => !hit.direction.is_incoming(),
        IndexedCombatDetailFilter::Incoming => hit.direction.is_incoming(),
        IndexedCombatDetailFilter::CharacterAttributed => {
            hit.direction.is_outgoing() && hit.char_known && !is_unbalance_damage_hit(hit)
        }
        IndexedCombatDetailFilter::CharacterDirect => {
            hit.direction.is_outgoing()
                && hit.char_known
                && !is_unbalance_damage_hit(hit)
                && hit.total_damage() > reaction_damage_for_hit(hit)
        }
        IndexedCombatDetailFilter::ReactionDamage => {
            hit.direction.is_outgoing() && hit.char_known && reaction_damage_for_hit(hit) > 0.0
        }
        IndexedCombatDetailFilter::SharedMechanics => {
            !hit.direction.is_incoming() && is_unbalance_damage_hit(hit)
        }
        IndexedCombatDetailFilter::Unattributed => {
            !(hit.direction.is_incoming()
                || is_unbalance_damage_hit(hit)
                || (hit.direction.is_outgoing() && hit.char_known))
        }
        IndexedCombatDetailFilter::QteType(value) => raw_qte_types(hit)
            .into_iter()
            .any(|raw| index.normalized_qte(raw) == value),
    }
}

fn add_hit_direction(summary: &mut HitDirectionSummary, hit: &Hit) {
    let damage = hit.total_damage();
    match hit.direction {
        HitDirection::Outgoing => {
            summary.outgoing_hits = summary.outgoing_hits.saturating_add(1);
            summary.outgoing_damage += damage;
        }
        HitDirection::Unknown => {
            summary.unknown_hits = summary.unknown_hits.saturating_add(1);
            summary.unknown_damage += damage;
        }
        HitDirection::Incoming => {
            summary.incoming_hits = summary.incoming_hits.saturating_add(1);
            summary.incoming_damage += damage;
        }
    }
}

fn remove_hit_direction(summary: &mut HitDirectionSummary, hit: &Hit) {
    let damage = hit.total_damage();
    match hit.direction {
        HitDirection::Outgoing => {
            summary.outgoing_hits = summary.outgoing_hits.saturating_sub(1);
            summary.outgoing_damage = (summary.outgoing_damage - damage).max(0.0);
        }
        HitDirection::Unknown => {
            summary.unknown_hits = summary.unknown_hits.saturating_sub(1);
            summary.unknown_damage = (summary.unknown_damage - damage).max(0.0);
        }
        HitDirection::Incoming => {
            summary.incoming_hits = summary.incoming_hits.saturating_sub(1);
            summary.incoming_damage = (summary.incoming_damage - damage).max(0.0);
        }
    }
}

fn qte_detail_contribution(hit: &Hit, qte: &str) -> (u64, f64) {
    let mut hits = 0_u64;
    let mut damage = 0.0;
    if hit.attack_type.as_deref() == Some(qte)
        || (qte == DETAIL_OVERFLOW_QTE
            && hit.attack_type.as_deref().is_some_and(|value| {
                is_qte_follow_up_damage_type(value) || is_unbalance_damage_hit(hit)
            }))
    {
        hits = hits.saturating_add(1);
        damage += hit.damage;
    }
    if hit.follow_up_damage > 0.0
        && (hit.follow_up_attack_type.as_deref() == Some(qte)
            || (qte == DETAIL_OVERFLOW_QTE
                && hit
                    .follow_up_attack_type
                    .as_deref()
                    .is_some_and(is_qte_follow_up_damage_type)))
    {
        hits = hits.saturating_add(1);
        damage += hit.follow_up_damage;
    }
    (hits, damage)
}

fn remove_detail_aggregate(
    values: &mut HashMap<String, IndexedDetailAggregate>,
    key: &str,
    damage: f64,
) -> bool {
    remove_detail_aggregate_counts(values, key, 1, damage)
}

fn remove_detail_aggregate_counts(
    values: &mut HashMap<String, IndexedDetailAggregate>,
    key: &str,
    hits: u64,
    damage: f64,
) -> bool {
    let remove = values.get_mut(key).is_some_and(|row| {
        row.hits = row.hits.saturating_sub(hits);
        row.damage = (row.damage - damage).max(0.0);
        row.hits == 0
    });
    if remove {
        values.remove(key);
    }
    remove
}

fn remove_inline_detail_aggregate(row: &mut IndexedDetailAggregate, hits: u64, damage: f64) {
    row.hits = row.hits.saturating_sub(hits);
    row.damage = (row.damage - damage).max(0.0);
    if row.hits == 0 {
        *row = IndexedDetailAggregate::default();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureQualitySource {
    Live,
    PcapngReplay,
    JsonReplay,
    #[default]
    Unknown,
}

impl CaptureQualitySource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Live => "实时抓包",
            Self::PcapngReplay => "PCAPNG 回放",
            Self::JsonReplay => "JSON 回放",
            Self::Unknown => "当前会话",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureQualitySummary {
    pub source: CaptureQualitySource,
    pub packet_count: usize,
    pub packets_with_hits: usize,
    pub hit_count: usize,
    pub outgoing_hits: u64,
    pub outgoing_damage: f64,
    pub unknown_direction_hits: u64,
    pub unknown_direction_damage: f64,
    pub incoming_hits: u64,
    pub incoming_damage: f64,
    pub unknown_character_count: usize,
    pub unknown_character_hits: u64,
    pub unmapped_skill_rows: usize,
    pub unmapped_skill_hits: u64,
    pub unmapped_gameplay_effect_count: usize,
    pub time_stop_event_count: u64,
    pub time_stop_interval_count: usize,
    pub abyss_event_count: u64,
    pub server_damage_corrections: u64,
    pub unattributed_server_damage_events: u64,
    pub unattributed_server_damage: f64,
}

/// Compact scalar snapshot retained for exact time-stop regression tests and
/// low-level adapters. Live diagnostics no longer use it to reconstruct hit
/// attribution; that data now comes from the reducer-maintained index.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg(feature = "desktop")]
#[allow(dead_code)]
pub(crate) struct CaptureQualityScalars {
    pub hits_generation: u64,
    pub packet_count: usize,
    pub packets_with_hits: usize,
    pub hit_count: usize,
    pub time_stop_event_count: u64,
    pub time_stop_interval_count: usize,
    pub abyss_event_count: u64,
    pub server_damage_corrections: u64,
    pub unattributed_server_damage_events: u64,
    pub unattributed_server_damage_bits: u64,
}

impl CaptureQualitySummary {
    pub fn redacted_text(&self) -> String {
        let mut text = String::new();
        let _ = writeln!(text, "NTE DPS TOOL 解析质量报告");
        let _ = writeln!(text, "统计来源：{}", self.source.label());
        let _ = writeln!(
            text,
            "封包：{} 个（含命中 {} 个）",
            self.packet_count, self.packets_with_hits
        );
        let _ = writeln!(text, "命中：{} 条", self.hit_count);
        let _ = writeln!(
            text,
            "方向：输出 {} 条 / 候选 {} 条 / 受击 {} 条",
            self.outgoing_hits, self.unknown_direction_hits, self.incoming_hits
        );
        let _ = writeln!(
            text,
            "伤害：输出 {:.0} / 候选 {:.0} / 受击 {:.0}",
            self.outgoing_damage, self.unknown_direction_damage, self.incoming_damage
        );
        let _ = writeln!(
            text,
            "未知角色：{} 个，{} 条命中",
            self.unknown_character_count, self.unknown_character_hits
        );
        let _ = writeln!(
            text,
            "待映射技能：{} 类，{} 条命中",
            self.unmapped_skill_rows, self.unmapped_skill_hits
        );
        let _ = writeln!(
            text,
            "未映射 GE：{} 个",
            self.unmapped_gameplay_effect_count
        );
        let _ = writeln!(
            text,
            "时停事件：{} 个，合并区间 {} 段",
            self.time_stop_event_count, self.time_stop_interval_count
        );
        let _ = writeln!(text, "深渊事件：{} 个", self.abyss_event_count);
        let _ = write!(
            text,
            "服务端伤害校准：{} 条；未归因观测：{} 条 / {:.0} 伤害",
            self.server_damage_corrections,
            self.unattributed_server_damage_events,
            self.unattributed_server_damage,
        );
        text
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CombatSessionSummary {
    pub duration_seconds: f64,
    pub dps_time_mode: DpsTimeBasis,
    pub total_damage: f64,
    pub total_dps: f64,
    pub total_damage_taken: f64,
    pub total_hits: u64,
    pub reaction_damage_separated: bool,
    pub damage_attribution: DamageAttributionSummary,
    pub characters: Vec<CombatSessionCharacterSummary>,
    pub skills: Vec<CombatSessionSkillSummary>,
    pub abyss: CombatSessionAbyssSummary,
    pub quality: CaptureQualitySummary,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DpsTimeBasis {
    #[default]
    #[serde(
        rename = "subtract_time_stop",
        alias = "time_stop_adjusted",
        alias = "Exclude Time Stop",
        alias = "扣除时停",
        alias = "時間停止を除外"
    )]
    SubtractTimeStop,
    #[serde(
        rename = "wall_clock",
        alias = "real_time",
        alias = "Real Time",
        alias = "实时",
        alias = "现实时间",
        alias = "実時間"
    )]
    WallClock,
}

impl DpsTimeBasis {
    pub const fn from_subtract_time_stop(subtract_time_stop: bool) -> Self {
        if subtract_time_stop {
            Self::SubtractTimeStop
        } else {
            Self::WallClock
        }
    }

    pub const fn subtracts_time_stop(self) -> bool {
        matches!(self, Self::SubtractTimeStop)
    }

    pub const fn protocol_code(self) -> &'static str {
        match self {
            Self::SubtractTimeStop => "subtract_time_stop",
            Self::WallClock => "wall_clock",
        }
    }

    /// English key; wrap with [`crate::storage::i18n::t`] at the display site.
    pub const fn label(self) -> &'static str {
        match self {
            Self::SubtractTimeStop => "Exclude Time Stop",
            Self::WallClock => "Real Time",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CombatSessionCharacterSummary {
    pub char_id: u32,
    pub name: String,
    pub hits: u64,
    pub damage: f64,
    pub dps: f64,
    pub damage_share_percent: f64,
    pub hits_taken: u64,
    pub damage_taken: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CombatSessionSkillSummary {
    pub char_id: u32,
    pub char_name: String,
    pub name: String,
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ability_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gameplay_effect_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damage_name: Option<String>,
    pub hits: u64,
    pub damage: f64,
    pub damage_share_percent: f64,
    pub is_follow_up: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CombatSessionAbyssSummary {
    pub detected: bool,
    pub floor: Option<u32>,
    pub active_half: Option<AbyssHalf>,
    pub success: bool,
    pub first_half: Option<CombatSessionAbyssHalfSummary>,
    pub second_half: Option<CombatSessionAbyssHalfSummary>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CombatSessionAbyssHalfSummary {
    pub half: AbyssHalf,
    pub duration_seconds: f64,
    pub total_damage: f64,
    pub total_dps: f64,
    pub damage_attribution: DamageAttributionSummary,
    pub characters: Vec<CombatSessionCharacterSummary>,
    pub skills: Vec<CombatSessionSkillSummary>,
}

#[allow(dead_code)]
pub fn summarize_timeline<'a, I>(hits: I, bucket_seconds: f64) -> TimelineSeries
where
    I: IntoIterator<Item = &'a Hit> + Clone,
{
    let mut start = None::<f64>;
    let mut end = None::<f64>;
    for hit in hits.clone() {
        if hit.direction.is_incoming() || !hit.timestamp.is_finite() {
            continue;
        }
        start = Some(start.map_or(hit.timestamp, |value| value.min(hit.timestamp)));
        end = Some(end.map_or(hit.timestamp, |value| value.max(hit.timestamp)));
    }
    summarize_timeline_with_time_stop(
        hits,
        &TimeStopTracker::default(),
        start,
        end,
        Vec::new(),
        TimelineAggregationOptions {
            bucket_seconds,
            subtract_time_stop: false,
            max_buckets: DEFAULT_MAX_TIMELINE_BUCKETS,
            max_roles_per_bucket: DEFAULT_MAX_TIMELINE_ROLES_PER_BUCKET,
            max_characters: DEFAULT_MAX_TIMELINE_CHARACTERS,
        },
    )
}

/// Default idle span (no outgoing damage) that separates one capture into
/// distinct combat segments.
pub const COMBAT_SEGMENT_GAP_SECONDS: f64 = 5.0;

/// One detected stretch of sustained combat within a capture. Offsets are
/// relative to the timeline start, matching [`TimelineSeries`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CombatSegment {
    pub start_offset: f64,
    pub end_offset: f64,
    pub duration: f64,
    pub total_damage: f64,
    pub hits: u64,
    pub dps: f64,
}

/// Split a [`TimelineSeries`] into combat segments separated by idle gaps longer
/// than `gap_seconds`. Derived from the already-aggregated buckets, so per-segment
/// damage matches the chart and the team totals exactly. This is purely
/// read-only — it never resets or mutates live combat state.
pub fn summarize_combat_segments(series: &TimelineSeries, gap_seconds: f64) -> Vec<CombatSegment> {
    let bucket_seconds = if series.bucket_seconds.is_finite() && series.bucket_seconds > 0.0 {
        series.bucket_seconds
    } else {
        1.0
    };
    let gap_seconds = if gap_seconds.is_finite() && gap_seconds > 0.0 {
        gap_seconds
    } else {
        COMBAT_SEGMENT_GAP_SECONDS
    };
    let gap_buckets = (gap_seconds / bucket_seconds).ceil().max(1.0) as usize;

    let mut segments: Vec<CombatSegment> = Vec::new();
    let mut current: Option<CombatSegment> = None;
    let mut empty_run = 0usize;
    for bucket in &series.buckets {
        let active = bucket.hits > 0 && bucket.damage > 0.0;
        if active {
            if empty_run >= gap_buckets
                && let Some(segment) = current.take()
            {
                segments.push(segment);
            }
            empty_run = 0;
            let segment = current.get_or_insert(CombatSegment {
                start_offset: bucket.start_offset,
                ..CombatSegment::default()
            });
            segment.end_offset = bucket.end_offset;
            segment.total_damage += bucket.damage;
            segment.hits += bucket.hits;
        } else {
            empty_run += 1;
        }
    }
    if let Some(segment) = current.take() {
        segments.push(segment);
    }
    for segment in &mut segments {
        segment.duration = (segment.end_offset - segment.start_offset).max(0.0);
        segment.dps = if segment.duration > 0.0 {
            segment.total_damage / segment.duration
        } else {
            0.0
        };
    }
    segments
}

pub fn summarize_skill_breakdown<'a>(
    hits: impl IntoIterator<Item = &'a Hit>,
    char_filter: Option<u32>,
) -> SkillBreakdown {
    let mut rows = HashMap::<SkillBreakdownKey, SkillBreakdownRow>::new();
    let mut unknown_characters = HashSet::<u32>::new();
    let mut unknown = UnknownAttributionSummary::default();
    let mut unmapped_gameplay_effects = HashMap::<u32, UnknownGameplayEffect>::new();
    let mut total_damage = 0.0;
    let mut total_hits = 0;

    for hit in hits.into_iter().filter(|hit| {
        !hit.direction.is_incoming() && char_filter.is_none_or(|char_id| hit.char_id == char_id)
    }) {
        if !hit.char_known {
            unknown_characters.insert(hit.char_id);
            unknown.unknown_character_hits += 1;
        }
        if hit.direction.is_unknown() {
            unknown.unknown_direction_hits += 1;
            unknown.unknown_direction_damage += hit.total_damage();
        }

        if hit.damage > 0.0 {
            let entry = SkillEntryRef::from_hit(hit, false);
            push_skill_breakdown_entry(&mut rows, hit, entry, hit.damage);
            total_damage += hit.damage;
            total_hits += 1;
            observe_unknown_skill(
                hit,
                hit.damage,
                &mut unknown,
                &mut unmapped_gameplay_effects,
            );
        }
        if hit.follow_up_damage > 0.0 {
            let entry = SkillEntryRef::from_hit(hit, true);
            push_skill_breakdown_entry(&mut rows, hit, entry, hit.follow_up_damage);
            total_damage += hit.follow_up_damage;
            total_hits += 1;
        }
    }

    let mut sorted_rows = rows.into_values().collect::<Vec<_>>();
    sorted_rows.sort_by(|left, right| {
        right
            .damage
            .total_cmp(&left.damage)
            .then_with(|| left.char_name.cmp(&right.char_name))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.is_follow_up.cmp(&right.is_follow_up))
    });
    unknown.unknown_character_count = unknown_characters.len();
    unknown.unmapped_skill_rows = sorted_rows
        .iter()
        .filter(|row| is_unmapped_skill_row(row))
        .count();
    unknown.unmapped_gameplay_effects = unmapped_gameplay_effects.into_values().collect();
    unknown
        .unmapped_gameplay_effects
        .sort_by_key(|effect| effect.index);

    SkillBreakdown {
        total_damage,
        total_hits,
        rows: sorted_rows,
        unknown,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SkillBreakdownKey {
    char_id: u32,
    name: String,
    category: String,
    ability_name: Option<String>,
    gameplay_effect_index: Option<u32>,
    gameplay_effect_name: Option<String>,
    is_follow_up: bool,
}

struct SkillEntryRef {
    name: String,
    category: String,
    ability_name: Option<String>,
    damage_name: Option<String>,
    gameplay_effect_index: Option<u32>,
    gameplay_effect_name: Option<String>,
    is_follow_up: bool,
}

impl SkillEntryRef {
    fn from_hit(hit: &Hit, is_follow_up: bool) -> Self {
        if is_follow_up {
            return Self {
                name: bounded_skill_text(
                    hit.follow_up_damage_name
                        .as_deref()
                        .or(hit.follow_up_attack_type.as_deref())
                        .unwrap_or("后续伤害"),
                    "Aggregated Overflow",
                ),
                category: bounded_skill_text(
                    hit.follow_up_attack_type.as_deref().unwrap_or("后续伤害"),
                    "Aggregated Overflow",
                ),
                ability_name: bounded_optional_skill_text(hit.ability_name.as_deref()),
                damage_name: bounded_optional_skill_text(hit.follow_up_damage_name.as_deref()),
                gameplay_effect_index: hit.gameplay_effect_index,
                gameplay_effect_name: bounded_optional_skill_text(
                    hit.gameplay_effect_name.as_deref(),
                ),
                is_follow_up,
            };
        }

        let damage_name = hit
            .damage_component
            .as_deref()
            .or(hit.damage_name.as_deref());
        Self {
            name: bounded_skill_text(
                hit.damage_component
                    .as_deref()
                    .or(hit.ability_name.as_deref())
                    .or(hit.gameplay_effect_name.as_deref())
                    .or(hit.damage_name.as_deref())
                    .or(hit.attack_type.as_deref())
                    .unwrap_or("待映射技能"),
                "Aggregated Overflow",
            ),
            category: bounded_skill_text(
                hit.attack_type.as_deref().unwrap_or("未归类"),
                "Aggregated Overflow",
            ),
            ability_name: bounded_optional_skill_text(hit.ability_name.as_deref()),
            damage_name: bounded_optional_skill_text(damage_name),
            gameplay_effect_index: hit.gameplay_effect_index,
            gameplay_effect_name: bounded_optional_skill_text(hit.gameplay_effect_name.as_deref()),
            is_follow_up,
        }
    }
}

fn bounded_skill_text(value: &str, fallback: &str) -> String {
    if value.len() <= MAX_INDEXED_SKILL_TEXT_BYTES {
        value.to_owned()
    } else {
        fallback.to_owned()
    }
}

fn bounded_optional_skill_text(value: Option<&str>) -> Option<String> {
    value.map(|value| bounded_skill_text(value, "Aggregated Overflow"))
}

fn push_skill_breakdown_entry(
    rows: &mut HashMap<SkillBreakdownKey, SkillBreakdownRow>,
    hit: &Hit,
    entry: SkillEntryRef,
    damage: f64,
) {
    if !damage.is_finite() || damage <= 0.0 {
        return;
    }
    let key = skill_breakdown_key(hit, &entry);
    let gameplay_effect_index = entry.gameplay_effect_index;
    let gameplay_effect_name = entry.gameplay_effect_name.clone();
    let row = rows.entry(key).or_insert_with(move || SkillBreakdownRow {
        char_id: hit.char_id,
        char_name: bounded_skill_text(&hit.char_name, "Aggregated Overflow"),
        name: entry.name,
        category: entry.category,
        ability_name: entry.ability_name,
        damage_name: entry.damage_name,
        gameplay_effect_index: entry.gameplay_effect_index,
        gameplay_effect_name: entry.gameplay_effect_name,
        is_follow_up: entry.is_follow_up,
        hits: 0,
        damage: 0.0,
    });
    if row.hits > 0 {
        if row.gameplay_effect_index != gameplay_effect_index {
            row.gameplay_effect_index = None;
        }
        if row.gameplay_effect_name != gameplay_effect_name {
            row.gameplay_effect_name = None;
        }
    }
    row.char_name = bounded_skill_text(&hit.char_name, "Aggregated Overflow");
    row.hits += 1;
    row.damage += damage;
}

fn skill_breakdown_key(hit: &Hit, entry: &SkillEntryRef) -> SkillBreakdownKey {
    let keep_gameplay_effect_key = entry.damage_name.is_none() && entry.ability_name.is_none();
    SkillBreakdownKey {
        char_id: hit.char_id,
        name: entry.name.clone(),
        category: entry.category.clone(),
        ability_name: entry.ability_name.clone(),
        gameplay_effect_index: if keep_gameplay_effect_key {
            entry.gameplay_effect_index
        } else {
            None
        },
        gameplay_effect_name: if keep_gameplay_effect_key {
            entry.gameplay_effect_name.clone()
        } else {
            None
        },
        is_follow_up: entry.is_follow_up,
    }
}

fn observe_unknown_skill(
    hit: &Hit,
    damage: f64,
    unknown: &mut UnknownAttributionSummary,
    unmapped_gameplay_effects: &mut HashMap<u32, UnknownGameplayEffect>,
) {
    if is_hit_skill_unmapped(hit) {
        unknown.unmapped_skill_hits += 1;
        unknown.unmapped_skill_damage += damage;
    }
    if let Some(index) = hit.gameplay_effect_index
        && hit.gameplay_effect_name.is_none()
    {
        let effect =
            unmapped_gameplay_effects
                .entry(index)
                .or_insert_with(|| UnknownGameplayEffect {
                    index,
                    ..Default::default()
                });
        effect.hits += 1;
        effect.damage += damage;
    }
}

fn is_hit_skill_unmapped(hit: &Hit) -> bool {
    hit.damage_name.is_none()
        && hit.damage_component.is_none()
        && hit.ability_name.is_none()
        && hit.gameplay_effect_name.is_none()
}

fn is_unmapped_skill_row(row: &SkillBreakdownRow) -> bool {
    !row.is_follow_up
        && row.damage_name.is_none()
        && row.ability_name.is_none()
        && row.gameplay_effect_name.is_none()
}

impl CharacterStats {
    pub fn has_hud_visible_hit(&self) -> bool {
        self.hud_visible_hits > 0
    }

    pub fn duration(&self) -> f64 {
        if self.hits > 1 {
            (self.last_hit - self.first_hit).max(0.001)
        } else {
            0.0
        }
    }

    pub fn for_reaction_damage_policy(&self, separate_reaction_damage: bool) -> Self {
        let mut projected = self.clone();
        let (hits, damage, first_hit, last_hit) = if separate_reaction_damage {
            (
                self.direct_hits,
                self.direct_damage,
                self.direct_first_hit,
                self.direct_last_hit,
            )
        } else {
            (
                self.attributed_hits,
                self.attributed_damage,
                self.attributed_first_hit,
                self.attributed_last_hit,
            )
        };
        projected.hits = hits;
        projected.damage = damage;
        projected.first_hit = if hits == 0 {
            0.0
        } else {
            first_hit.unwrap_or_default()
        };
        projected.last_hit = if hits == 0 {
            0.0
        } else {
            last_hit.unwrap_or_default()
        };
        projected
    }
}

pub const REACTION_DAMAGE_TYPES: [&str; 8] = [
    "创生花",
    "覆纹",
    "延滞",
    "黯星",
    "浊燃",
    "浸染",
    "盈蓄",
    "失谐",
];

pub fn is_reaction_damage_type(attack_type: &str) -> bool {
    REACTION_DAMAGE_TYPES.contains(&attack_type)
}

pub fn is_qte_follow_up_damage_type(attack_type: &str) -> bool {
    is_reaction_damage_type(attack_type)
}

pub fn is_qte_follow_up_damage_hit(hit: &Hit) -> bool {
    hit.follow_up_attack_type
        .as_deref()
        .is_some_and(is_qte_follow_up_damage_type)
        || (!hit.char_known
            && hit
                .attack_type
                .as_deref()
                .is_some_and(is_qte_follow_up_damage_type))
}

pub fn reaction_damage_for_hit(hit: &Hit) -> f64 {
    let primary = if hit
        .attack_type
        .as_deref()
        .is_some_and(is_reaction_damage_type)
    {
        hit.damage
    } else {
        0.0
    };
    let follow_up = if hit
        .follow_up_attack_type
        .as_deref()
        .is_some_and(is_reaction_damage_type)
    {
        hit.follow_up_damage
    } else {
        0.0
    };
    primary + follow_up
}

fn direct_damage_for_hit(hit: &Hit) -> f64 {
    (hit.total_damage() - reaction_damage_for_hit(hit)).max(0.0)
}

/// The `attack_type` classification used for "倾陷伤害" (Unbalance/Tenacity
/// burst) ticks — see [`is_unbalance_damage_hit`].
pub const UNBALANCE_ATTACK_TYPE: &str = "倾陷伤害";

/// Whether `hit` is a "倾陷伤害" (Unbalance/Tenacity burst) tick. The game
/// attributes the whole burst to whichever character happens to be on-field
/// when the team's shared stagger gauge pops, not to whoever actually filled
/// it — see issue #15. Kept in the team `total_damage` (it did reduce the
/// target's HP) but excluded from any single character's personal totals so
/// it can't inflate one character's ranking/DPS share.
pub fn is_unbalance_damage_hit(hit: &Hit) -> bool {
    hit.attack_type.as_deref() == Some(UNBALANCE_ATTACK_TYPE)
        || hit
            .damage_name
            .as_deref()
            .is_some_and(|damage_name| damage_name.contains("倾陷"))
}

fn summarize_damage_attribution<'a>(
    total_damage: f64,
    max_hp_reduction: f64,
    rows: impl IntoIterator<Item = &'a CharacterStats>,
) -> DamageAttributionSummary {
    let mut retained_character_damage = 0.0;
    let mut attributed_damage = 0.0;
    let mut direct_damage = 0.0;
    for row in rows {
        retained_character_damage += row.damage;
        attributed_damage += row.attributed_damage;
        direct_damage += row.direct_damage;
    }
    DamageAttributionSummary {
        total_damage,
        max_hp_reduction,
        character_direct_damage: direct_damage,
        character_reaction_damage: (attributed_damage - direct_damage).max(0.0),
        shared_damage: (total_damage - retained_character_damage).max(0.0),
        unattributed_damage: (retained_character_damage - attributed_damage).max(0.0),
    }
}

#[allow(clippy::too_many_arguments)]
fn update_combat_totals(
    stats: &mut HashMap<u32, CharacterStats>,
    compact_timeline: &mut CompactTimelineIndex,
    started_at: &mut Option<f64>,
    ended_at: &mut Option<f64>,
    total_damage: &mut f64,
    total_damage_taken: &mut f64,
    max_hp_reduction: &mut f64,
    hit: &Hit,
) {
    compact_timeline.observe_hit(hit);
    let stats_char_id = combat_stats_character_id(stats, hit.char_id);
    let overflow = stats_char_id == COMBAT_STATS_OVERFLOW_CHARACTER_ID;
    let row = stats
        .entry(stats_char_id)
        .or_insert_with(|| CharacterStats {
            char_id: stats_char_id,
            name: if overflow {
                "Aggregated Overflow".to_owned()
            } else {
                hit.char_name.clone()
            },
            first_hit: hit.timestamp,
            last_hit: hit.timestamp,
            ..Default::default()
        });
    if !overflow {
        row.name.clone_from(&hit.char_name);
    }
    if hit.char_known || !is_qte_follow_up_damage_hit(hit) {
        row.hud_visible_hits = row.hud_visible_hits.saturating_add(1);
    }
    let damage = hit.total_damage();
    if hit.direction.is_incoming() {
        row.hits_taken += 1;
        row.damage_taken += damage;
        *total_damage_taken += damage;
        return;
    }

    *started_at = Some(started_at.map_or(hit.timestamp, |value| value.min(hit.timestamp)));
    *ended_at = Some(ended_at.map_or(hit.timestamp, |value| value.max(hit.timestamp)));
    *total_damage += damage;
    *max_hp_reduction += hit.max_hp_reduction.max(0.0);
    if is_unbalance_damage_hit(hit) {
        return;
    }
    if row.hits == 0 {
        row.first_hit = hit.timestamp;
        row.last_hit = hit.timestamp;
    } else {
        row.first_hit = row.first_hit.min(hit.timestamp);
        row.last_hit = row.last_hit.max(hit.timestamp);
    }
    row.hits += 1;
    row.damage += damage;
    if matches!(hit.direction, HitDirection::Outgoing) && hit.char_known {
        if row.attributed_hits == 0 {
            row.attributed_first_hit = Some(hit.timestamp);
            row.attributed_last_hit = Some(hit.timestamp);
        } else {
            row.attributed_first_hit = Some(
                row.attributed_first_hit
                    .map_or(hit.timestamp, |first_hit| first_hit.min(hit.timestamp)),
            );
            row.attributed_last_hit = Some(
                row.attributed_last_hit
                    .map_or(hit.timestamp, |last_hit| last_hit.max(hit.timestamp)),
            );
        }
        row.attributed_hits += 1;
        row.attributed_damage += damage;

        let direct_damage = direct_damage_for_hit(hit);
        if direct_damage > 0.0 {
            if row.direct_hits == 0 {
                row.direct_first_hit = Some(hit.timestamp);
                row.direct_last_hit = Some(hit.timestamp);
            } else {
                row.direct_first_hit = Some(
                    row.direct_first_hit
                        .map_or(hit.timestamp, |first_hit| first_hit.min(hit.timestamp)),
                );
                row.direct_last_hit = Some(
                    row.direct_last_hit
                        .map_or(hit.timestamp, |last_hit| last_hit.max(hit.timestamp)),
                );
            }
            row.direct_hits += 1;
            row.direct_damage += direct_damage;
        }
    }
}

fn combat_stats_character_id(stats: &HashMap<u32, CharacterStats>, char_id: u32) -> u32 {
    if char_id != COMBAT_STATS_OVERFLOW_CHARACTER_ID
        && (stats.contains_key(&char_id)
            || (!stats.contains_key(&COMBAT_STATS_OVERFLOW_CHARACTER_ID)
                && stats.len() < MAX_COMBAT_STAT_ROWS - 1)
            || (stats.contains_key(&COMBAT_STATS_OVERFLOW_CHARACTER_ID)
                && stats.len() < MAX_COMBAT_STAT_ROWS))
    {
        char_id
    } else {
        COMBAT_STATS_OVERFLOW_CHARACTER_ID
    }
}

/// Applies the only aggregate delta caused when exact enemy telemetry promotes
/// an already retained `Unknown` hit to `Outgoing`. Team damage, hit count,
/// combat clock and timeline membership are direction-equivalent for those two
/// states; only confirmed character attribution/direct damage changes. The
/// hit was already admitted into `stats`, so this is constant work even for an
/// arbitrarily long combat.
fn promote_unknown_direction_totals(
    stats: &mut HashMap<u32, CharacterStats>,
    hit: HitAggregateContribution,
) {
    if !hit.attributed {
        return;
    }
    let stats_char_id = if stats.contains_key(&hit.char_id) {
        hit.char_id
    } else {
        COMBAT_STATS_OVERFLOW_CHARACTER_ID
    };
    let Some(row) = stats.get_mut(&stats_char_id) else {
        // This can only occur for an internally inconsistent state. External
        // telemetry remains fail-closed instead of triggering an O(N) recovery
        // scan while the live capture lock is held.
        return;
    };

    row.attributed_hits = row.attributed_hits.saturating_add(1);
    add_damage_delta(&mut row.attributed_damage, hit.total_damage);
    row.attributed_first_hit = Some(
        row.attributed_first_hit
            .map_or(hit.timestamp, |value| value.min(hit.timestamp)),
    );
    row.attributed_last_hit = Some(
        row.attributed_last_hit
            .map_or(hit.timestamp, |value| value.max(hit.timestamp)),
    );

    if hit.direct_counted {
        row.direct_hits = row.direct_hits.saturating_add(1);
        add_damage_delta(&mut row.direct_damage, hit.direct_damage);
        row.direct_first_hit = Some(
            row.direct_first_hit
                .map_or(hit.timestamp, |value| value.min(hit.timestamp)),
        );
        row.direct_last_hit = Some(
            row.direct_last_hit
                .map_or(hit.timestamp, |value| value.max(hit.timestamp)),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn rebuild_combat_totals(
    hits: &VecDeque<Hit>,
    stats: &mut HashMap<u32, CharacterStats>,
    compact_timeline: &mut CompactTimelineIndex,
    started_at: &mut Option<f64>,
    ended_at: &mut Option<f64>,
    total_damage: &mut f64,
    total_damage_taken: &mut f64,
    max_hp_reduction: &mut f64,
) {
    #[cfg(test)]
    COMBAT_TOTAL_REBUILD_COUNT.with(|count| count.set(count.get().saturating_add(1)));
    stats.clear();
    *compact_timeline = CompactTimelineIndex::default();
    *started_at = None;
    *ended_at = None;
    *total_damage = 0.0;
    *total_damage_taken = 0.0;
    *max_hp_reduction = 0.0;
    for hit in hits {
        update_combat_totals(
            stats,
            compact_timeline,
            started_at,
            ended_at,
            total_damage,
            total_damage_taken,
            max_hp_reduction,
            hit,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn rebuild_all_combat_indexes(
    hits: &VecDeque<Hit>,
    stats: &mut HashMap<u32, CharacterStats>,
    compact_timeline: &mut CompactTimelineIndex,
    skill_breakdown_index: &mut SkillBreakdownIndex,
    combat_detail_index: &mut CombatDetailIndex,
    started_at: &mut Option<f64>,
    ended_at: &mut Option<f64>,
    total_damage: &mut f64,
    total_damage_taken: &mut f64,
    max_hp_reduction: &mut f64,
) {
    stats.clear();
    *compact_timeline = CompactTimelineIndex::default();
    *skill_breakdown_index = SkillBreakdownIndex::default();
    *combat_detail_index = CombatDetailIndex::default();
    *started_at = None;
    *ended_at = None;
    *total_damage = 0.0;
    *total_damage_taken = 0.0;
    *max_hp_reduction = 0.0;
    let stable_end = hits.len().saturating_sub(RECENT_HIT_MUTATION_WINDOW);
    for (position, hit) in hits.iter().enumerate() {
        #[cfg(test)]
        BULK_COMBAT_REBUILD_VISITS.with(|visits| {
            visits.set(visits.get().saturating_add(1));
        });
        update_combat_totals(
            stats,
            compact_timeline,
            started_at,
            ended_at,
            total_damage,
            total_damage_taken,
            max_hp_reduction,
            hit,
        );
        skill_breakdown_index.observe_hit(hit);
        combat_detail_index.observe_hit(position, hit);
        if position < stable_end {
            combat_detail_index.promote_stable_hit(hit);
        }
    }
}

fn add_damage_delta(value: &mut f64, delta: f64) {
    *value += delta;
    // Repeated floating-point corrections can leave a negative zero or a tiny
    // residual even though every authoritative damage input is non-negative.
    if value.abs() <= 1e-9 {
        *value = 0.0;
    }
}

fn direct_totals_for_character(
    hits: &VecDeque<Hit>,
    stats: &HashMap<u32, CharacterStats>,
    stats_char_id: u32,
) -> (u64, f64, Option<f64>, Option<f64>) {
    let mut count = 0_u64;
    let mut damage = 0.0;
    let mut first: Option<f64> = None;
    let mut last: Option<f64> = None;
    for hit in hits.iter().filter(|hit| {
        (if stats_char_id == COMBAT_STATS_OVERFLOW_CHARACTER_ID {
            hit.char_id == COMBAT_STATS_OVERFLOW_CHARACTER_ID || !stats.contains_key(&hit.char_id)
        } else {
            hit.char_id == stats_char_id
        }) && !hit.direction.is_incoming()
            && !is_unbalance_damage_hit(hit)
            && matches!(hit.direction, HitDirection::Outgoing)
            && hit.char_known
    }) {
        let direct_damage = direct_damage_for_hit(hit);
        if direct_damage <= 0.0 {
            continue;
        }
        count = count.saturating_add(1);
        damage += direct_damage;
        first = Some(first.map_or(hit.timestamp, |value| value.min(hit.timestamp)));
        last = Some(last.map_or(hit.timestamp, |value| value.max(hit.timestamp)));
    }
    (count, damage, first, last)
}

/// Applies a mutation that cannot change hit ownership, direction or timestamp
/// by adjusting only that hit's aggregate contribution. A full rebuild remains
/// available for trim/import/recovery, but normal follow-up/correction traffic
/// is O(1); the only scan below is the rare direct-damage true -> false boundary
/// needed to recover that character's first/last direct timestamp.
#[allow(clippy::too_many_arguments)]
fn apply_combat_totals_delta(
    hits: &VecDeque<Hit>,
    stats: &mut HashMap<u32, CharacterStats>,
    compact_timeline: &mut CompactTimelineIndex,
    started_at: &mut Option<f64>,
    ended_at: &mut Option<f64>,
    total_damage: &mut f64,
    total_damage_taken: &mut f64,
    max_hp_reduction: &mut f64,
    mutation: HitAggregateMutation,
) {
    let before = mutation.before;
    let after = mutation.after;
    if before.char_id != after.char_id
        || before.timestamp.to_bits() != after.timestamp.to_bits()
        || before.incoming != after.incoming
        || before.character_counted != after.character_counted
        || before.attributed != after.attributed
    {
        // Internal recovery only: the public mutation helpers do not alter any
        // of these structural fields. Rebuild rather than propagating a partial
        // aggregate if a future mutation violates that contract.
        rebuild_combat_totals(
            hits,
            stats,
            compact_timeline,
            started_at,
            ended_at,
            total_damage,
            total_damage_taken,
            max_hp_reduction,
        );
        return;
    }
    let damage_delta = after.total_damage - before.total_damage;
    let stats_char_id = if stats.contains_key(&after.char_id) {
        after.char_id
    } else {
        COMBAT_STATS_OVERFLOW_CHARACTER_ID
    };
    let recomputed_direct_totals =
        matches!((before.direct_counted, after.direct_counted), (true, false))
            .then(|| direct_totals_for_character(hits, stats, stats_char_id));
    let Some(row) = stats.get_mut(&stats_char_id) else {
        rebuild_combat_totals(
            hits,
            stats,
            compact_timeline,
            started_at,
            ended_at,
            total_damage,
            total_damage_taken,
            max_hp_reduction,
        );
        return;
    };

    compact_timeline.apply_mutation(before, after);
    match (before.hud_visible, after.hud_visible) {
        (false, true) => {
            row.hud_visible_hits = row.hud_visible_hits.saturating_add(1);
        }
        (true, false) => {
            row.hud_visible_hits = row.hud_visible_hits.saturating_sub(1);
        }
        _ => {}
    }

    if after.incoming {
        add_damage_delta(&mut row.damage_taken, damage_delta);
        add_damage_delta(total_damage_taken, damage_delta);
        return;
    }

    add_damage_delta(total_damage, damage_delta);
    add_damage_delta(
        max_hp_reduction,
        after.max_hp_reduction - before.max_hp_reduction,
    );
    if !after.character_counted {
        return;
    }
    add_damage_delta(&mut row.damage, damage_delta);
    if !after.attributed {
        return;
    }
    add_damage_delta(&mut row.attributed_damage, damage_delta);

    match (before.direct_counted, after.direct_counted) {
        (false, true) => {
            row.direct_hits = row.direct_hits.saturating_add(1);
            add_damage_delta(&mut row.direct_damage, after.direct_damage);
            row.direct_first_hit = Some(
                row.direct_first_hit
                    .map_or(after.timestamp, |value| value.min(after.timestamp)),
            );
            row.direct_last_hit = Some(
                row.direct_last_hit
                    .map_or(after.timestamp, |value| value.max(after.timestamp)),
            );
        }
        (true, false) => {
            if let Some((count, damage, first, last)) = recomputed_direct_totals {
                row.direct_hits = count;
                row.direct_damage = damage;
                row.direct_first_hit = first;
                row.direct_last_hit = last;
            }
        }
        (true, true) => {
            add_damage_delta(
                &mut row.direct_damage,
                after.direct_damage - before.direct_damage,
            );
        }
        (false, false) => {}
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbyssHalf {
    #[default]
    #[serde(alias = "Ascending Line", alias = "上行线", alias = "上りライン")]
    First,
    #[serde(alias = "Descending Line", alias = "下行线", alias = "下りライン")]
    Second,
}

impl AbyssHalf {
    /// English key; wrap with [`crate::storage::i18n::t`] at the display site.
    pub fn label(self) -> &'static str {
        match self {
            Self::First => "Ascending Line",
            Self::Second => "Descending Line",
        }
    }
}

#[derive(Clone, Debug)]
pub enum AbyssEvent {
    RestartDetected {
        timestamp: f64,
    },
    Stage {
        timestamp: f64,
        #[allow(dead_code)]
        cycle: Option<u32>,
        floor: Option<u32>,
        half: AbyssHalf,
        allow_late_backfill: bool,
    },
    Success {
        timestamp: f64,
    },
    Exit {
        timestamp: f64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TimeStopEvent {
    GamePauseStarted {
        timestamp: f64,
        pause_type_mask: u32,
    },
    GamePauseEnded {
        timestamp: f64,
        pause_type_mask: u32,
    },
}

/// Runtime health of the authoritative game-side combat-clock provider. This
/// is separate from the user's configured DPS basis: `TimeStopAdjusted` must
/// never imply that the provider is actually connected and producing data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CombatClockRuntimeHealth {
    #[default]
    Unknown,
    /// Replay/import supplied authoritative recorded pause transitions.
    Recorded,
    Available,
    /// Provider/controller responded, but the current generation has not
    /// produced a pause-valid combat-clock sample (or validity was lost).
    DataUnavailable,
    ProviderUnavailable,
    ModDisabled,
    InvalidResponse,
}

impl CombatClockRuntimeHealth {
    pub const fn supports_time_stop_adjustment(self) -> bool {
        matches!(self, Self::Recorded | Self::Available)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TimeStopInterval {
    start: f64,
    end: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ArchivedTimeStopIntervals {
    start: f64,
    end: f64,
    frozen_duration: f64,
    count: u64,
}

impl ArchivedTimeStopIntervals {
    /// Returns the archived frozen time inside a query window. Whole-prefix
    /// queries are exact (the normal combat/session path). Once the bounded
    /// interval budget has compacted old bands, partial queries use a
    /// deterministic density projection and are surfaced as one synthetic
    /// interval rather than re-expanding O(event_count) state.
    fn frozen_between(self, start: f64, end: f64) -> f64 {
        if !start.is_finite()
            || !end.is_finite()
            || end <= start
            || self.end <= self.start
            || self.frozen_duration <= 0.0
        {
            return 0.0;
        }
        let overlap_start = start.max(self.start);
        let overlap_end = end.min(self.end);
        if overlap_end <= overlap_start {
            return 0.0;
        }
        if start <= self.start && end >= self.end {
            return self.frozen_duration.min(self.end - self.start);
        }
        let density = (self.frozen_duration / (self.end - self.start)).clamp(0.0, 1.0);
        ((overlap_end - overlap_start) * density).min(overlap_end - overlap_start)
    }

    fn projected_interval(self, start: f64, end: f64) -> Option<TimeStopInterval> {
        let duration = self.frozen_between(start, end);
        if duration <= 0.0 {
            return None;
        }
        let projected_end = end.min(self.end);
        Some(TimeStopInterval {
            start: (projected_end - duration).max(start),
            end: projected_end,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct TimeStopTracker {
    intervals: VecDeque<TimeStopInterval>,
    archived: Option<ArchivedTimeStopIntervals>,
    active_game_pause: Option<(f64, u32)>,
    latest_game_pause_transition: Option<f64>,
    event_count: u64,
}

impl TimeStopTracker {
    fn apply_event(&mut self, event: &TimeStopEvent) {
        match event {
            TimeStopEvent::GamePauseStarted {
                timestamp,
                pause_type_mask,
            } => {
                if !timestamp.is_finite() {
                    return;
                }
                match &mut self.active_game_pause {
                    Some((start, active_mask)) => {
                        *start = start.min(*timestamp);
                        *active_mask |= *pause_type_mask;
                    }
                    None => {
                        self.active_game_pause = Some((*timestamp, *pause_type_mask));
                    }
                }
                self.record_game_pause_transition(*timestamp);
            }
            TimeStopEvent::GamePauseEnded { timestamp, .. } => {
                let Some((start, _)) = self.active_game_pause.take() else {
                    return;
                };
                self.event_count = self.event_count.saturating_add(1);
                self.push_interval(start, *timestamp);
                self.record_game_pause_transition(*timestamp);
            }
        }
    }

    fn record_game_pause_transition(&mut self, timestamp: f64) {
        if timestamp.is_finite() {
            self.latest_game_pause_transition = Some(
                self.latest_game_pause_transition
                    .map_or(timestamp, |value| value.max(timestamp)),
            );
        }
    }

    fn push_interval(&mut self, start: f64, end: f64) {
        if !start.is_finite() || !end.is_finite() || end <= start {
            return;
        }
        if let Some(last) = self.intervals.back_mut()
            && start <= last.end
        {
            last.end = last.end.max(end);
            return;
        }
        if self.intervals.len() == MAX_RETAINED_TIME_STOP_INTERVALS
            && let Some(expired) = self.intervals.pop_front()
        {
            let archived = self.archived.get_or_insert(ArchivedTimeStopIntervals {
                start: expired.start,
                end: expired.end,
                frozen_duration: 0.0,
                count: 0,
            });
            archived.start = archived.start.min(expired.start);
            archived.end = archived.end.max(expired.end);
            archived.frozen_duration += expired.end - expired.start;
            archived.count = archived.count.saturating_add(1);
        }
        self.intervals.push_back(TimeStopInterval { start, end });
    }

    fn frozen_between(&self, start: f64, end: f64) -> f64 {
        self.intervals_between(start, end)
            .into_iter()
            .map(|interval| interval.end - interval.start)
            .sum::<f64>()
    }

    fn latest_game_pause_transition(&self) -> Option<f64> {
        self.latest_game_pause_transition
    }

    fn compacted_interval_count(&self) -> u64 {
        self.archived.map_or(0, |archived| archived.count)
    }

    fn intervals_between(&self, start: f64, end: f64) -> Vec<TimeStopInterval> {
        if !start.is_finite() || !end.is_finite() || end <= start {
            return Vec::new();
        }
        let intervals = self
            .archived
            .and_then(|archived| archived.projected_interval(start, end))
            .into_iter()
            .chain(
                self.intervals
                    .iter()
                    .copied()
                    .chain(
                        self.active_game_pause
                            .map(|(active_start, _)| TimeStopInterval {
                                start: active_start,
                                end,
                            }),
                    ),
            )
            .filter_map(|interval| Self::clip_interval(interval, start, end))
            .collect::<Vec<_>>();
        Self::merge_intervals(intervals)
    }

    /// Counts the same clipped union as [`Self::intervals_between`] without
    /// allocating or sorting a temporary vector. Capture events normally
    /// arrive in timestamp order, so the first pass is linear. The allocation-
    /// free fallback preserves exact semantics for older or out-of-order
    /// replay fixtures.
    #[cfg(feature = "desktop")]
    #[allow(dead_code)]
    fn interval_count_between(&self, start: f64, end: f64) -> usize {
        if !start.is_finite() || !end.is_finite() || end <= start {
            return 0;
        }

        let visit = |visitor: &mut dyn FnMut(TimeStopInterval)| {
            if let Some(interval) = self
                .archived
                .and_then(|archived| archived.projected_interval(start, end))
            {
                visitor(interval);
            }
            for interval in self.intervals.iter().copied() {
                if let Some(interval) = Self::clip_interval(interval, start, end) {
                    visitor(interval);
                }
            }
            if let Some((active_start, _)) = self.active_game_pause
                && let Some(interval) = Self::clip_interval(
                    TimeStopInterval {
                        start: active_start,
                        end,
                    },
                    start,
                    end,
                )
            {
                visitor(interval);
            }
        };

        let mut previous_start = None;
        let mut sorted = true;
        visit(&mut |interval| {
            if previous_start.is_some_and(|previous| interval.start < previous) {
                sorted = false;
            }
            previous_start = Some(interval.start);
        });
        if sorted {
            let mut count = 0_usize;
            let mut merged_end = None::<f64>;
            visit(&mut |interval| match merged_end {
                Some(current_end) if interval.start <= current_end => {
                    merged_end = Some(current_end.max(interval.end));
                }
                Some(_) => {
                    count = count.saturating_add(1);
                    merged_end = Some(interval.end);
                }
                None => merged_end = Some(interval.end),
            });
            return count.saturating_add(usize::from(merged_end.is_some()));
        }

        // No-allocation union count for out-of-order input. Find the next
        // component seed, then repeatedly extend its right edge until every
        // touching/overlapping interval has been consumed.
        let mut count = 0_usize;
        let mut previous_component_end = None::<f64>;
        loop {
            let mut seed = None::<TimeStopInterval>;
            visit(&mut |interval| {
                if previous_component_end.is_some_and(|end| interval.start <= end) {
                    return;
                }
                let replace = seed.is_none_or(|current| {
                    interval.start < current.start
                        || (interval.start == current.start && interval.end > current.end)
                });
                if replace {
                    seed = Some(interval);
                }
            });
            let Some(seed) = seed else {
                break;
            };
            let mut component_end = seed.end;
            loop {
                let before = component_end;
                visit(&mut |interval| {
                    if interval.start <= component_end && interval.end > component_end {
                        component_end = interval.end;
                    }
                });
                if component_end == before {
                    break;
                }
            }
            count = count.saturating_add(1);
            previous_component_end = Some(component_end);
        }
        count
    }

    fn clip_interval(interval: TimeStopInterval, start: f64, end: f64) -> Option<TimeStopInterval> {
        let clipped_start = interval.start.max(start);
        let clipped_end = interval.end.min(end);
        (clipped_end > clipped_start).then_some(TimeStopInterval {
            start: clipped_start,
            end: clipped_end,
        })
    }

    fn merge_intervals(mut intervals: Vec<TimeStopInterval>) -> Vec<TimeStopInterval> {
        intervals.sort_by(|left, right| left.start.total_cmp(&right.start));

        let mut merged_intervals = Vec::new();
        let mut merged: Option<TimeStopInterval> = None;
        for interval in intervals {
            match merged {
                Some(mut current) if interval.start <= current.end => {
                    current.end = current.end.max(interval.end);
                    merged = Some(current);
                }
                Some(current) => {
                    merged_intervals.push(current);
                    merged = Some(interval);
                }
                None => merged = Some(interval),
            }
        }
        if let Some(current) = merged {
            merged_intervals.push(current);
        }
        merged_intervals
    }
}

fn compact_time_stop_event_prefix(events: &mut Vec<TimeStopEvent>) {
    let drain_count = TIME_STOP_EVENT_COMPACTION_CHUNK.min(events.len());
    if drain_count == 0 {
        return;
    }
    let drained = events.drain(..drain_count).collect::<Vec<_>>();
    let mut tracker = TimeStopTracker::default();
    for event in &drained {
        tracker.apply_event(event);
    }
    let intervals = TimeStopTracker::merge_intervals(tracker.intervals.iter().copied().collect());
    let frozen_duration = intervals
        .iter()
        .map(|interval| interval.end - interval.start)
        .sum::<f64>();
    let mut prefix = Vec::with_capacity(3);
    if frozen_duration > 0.0
        && let Some(compacted_end) = intervals.last().map(|interval| interval.end)
    {
        prefix.push(TimeStopEvent::GamePauseStarted {
            timestamp: compacted_end - frozen_duration,
            pause_type_mask: 0,
        });
        prefix.push(TimeStopEvent::GamePauseEnded {
            timestamp: compacted_end,
            pause_type_mask: 0,
        });
    }
    if let Some((active_start, active_mask)) = tracker.active_game_pause {
        prefix.push(TimeStopEvent::GamePauseStarted {
            timestamp: active_start,
            pause_type_mask: active_mask,
        });
    }
    prefix.append(events);
    *events = prefix;
}

#[derive(Clone, Debug, Default)]
pub struct PartyCombatState {
    pub hits: VecDeque<Hit>,
    pub hits_generation: u64,
    pub stats: HashMap<u32, CharacterStats>,
    pub started_at: Option<f64>,
    pub ended_at: Option<f64>,
    pub total_damage: f64,
    pub total_damage_taken: f64,
    pub max_hp_reduction: f64,
    compact_timeline: CompactTimelineIndex,
    skill_breakdown_index: SkillBreakdownIndex,
    combat_detail_index: CombatDetailIndex,
    time_stop: TimeStopTracker,
}

impl PartyCombatState {
    pub fn push_hit(&mut self, hit: Hit) {
        let position = self.hits.len();
        update_combat_totals(
            &mut self.stats,
            &mut self.compact_timeline,
            &mut self.started_at,
            &mut self.ended_at,
            &mut self.total_damage,
            &mut self.total_damage_taken,
            &mut self.max_hp_reduction,
            &hit,
        );
        self.skill_breakdown_index.observe_hit(&hit);
        if let Some(stable_hit) = position
            .checked_sub(RECENT_HIT_MUTATION_WINDOW)
            .and_then(|position| self.hits.get(position))
        {
            self.combat_detail_index.promote_stable_hit(stable_hit);
        }
        self.combat_detail_index.observe_hit(position, &hit);
        self.hits.push_back(hit);
        reconcile_latest_overkill_interval(&mut self.hits);
        self.hits_generation = self.hits_generation.wrapping_add(1);
        self.sync_clock_with_time_stops();
    }

    /// Rehydrates an owned History half without replaying every hit through
    /// the live mutation path. All runtime read indexes are rebuilt together
    /// in one pass; no hit strings are cloned.
    pub(crate) fn replace_hits_bulk(&mut self, hits: Vec<Hit>) {
        self.hits = hits.into();
        self.hits_generation = u64::try_from(self.hits.len()).unwrap_or(u64::MAX);
        rebuild_all_combat_indexes(
            &self.hits,
            &mut self.stats,
            &mut self.compact_timeline,
            &mut self.skill_breakdown_index,
            &mut self.combat_detail_index,
            &mut self.started_at,
            &mut self.ended_at,
            &mut self.total_damage,
            &mut self.total_damage_taken,
            &mut self.max_hp_reduction,
        );
        self.sync_clock_with_time_stops();
    }

    fn apply_follow_up_at(&mut self, locator: HitLocator, follow_up: &HitFollowUp) -> bool {
        let position = find_recent_hit_position(&self.hits, locator);
        let before = find_recent_hit(&self.hits, locator).cloned();
        let mutation = apply_follow_up_to_recent_hit(&mut self.hits, locator, follow_up);
        if let Some(mutation) = mutation {
            if let (Some(before), Some(after)) = (before, find_recent_hit(&self.hits, locator)) {
                self.skill_breakdown_index.replace_hit(&before, after);
                if let Some(position) = position {
                    self.combat_detail_index
                        .replace_hit(position, &before, after);
                }
            }
            self.hits_generation = self.hits_generation.wrapping_add(1);
            apply_combat_totals_delta(
                &self.hits,
                &mut self.stats,
                &mut self.compact_timeline,
                &mut self.started_at,
                &mut self.ended_at,
                &mut self.total_damage,
                &mut self.total_damage_taken,
                &mut self.max_hp_reduction,
                mutation,
            );
            return true;
        }
        false
    }

    fn apply_damage_correction_at(
        &mut self,
        locator: HitLocator,
        correction: &HitDamageCorrection,
    ) -> bool {
        let position = find_recent_hit_position(&self.hits, locator);
        let before = find_recent_hit(&self.hits, locator).cloned();
        let mutation = apply_damage_correction_to_recent_hit(&mut self.hits, locator, correction);
        if let Some(mutation) = mutation {
            if let (Some(before), Some(after)) = (before, find_recent_hit(&self.hits, locator)) {
                self.skill_breakdown_index.replace_hit(&before, after);
                if let Some(position) = position {
                    self.combat_detail_index
                        .replace_hit(position, &before, after);
                }
            }
            self.hits_generation = self.hits_generation.wrapping_add(1);
            apply_combat_totals_delta(
                &self.hits,
                &mut self.stats,
                &mut self.compact_timeline,
                &mut self.started_at,
                &mut self.ended_at,
                &mut self.total_damage,
                &mut self.total_damage_taken,
                &mut self.max_hp_reduction,
                mutation,
            );
            return true;
        }
        false
    }

    fn apply_enemy_target_projection_result(
        &mut self,
        result: EnemyTargetProjectionResult,
        hit_damage: f64,
        promoted_hit: Option<HitAggregateContribution>,
    ) {
        if result.changed {
            self.hits_generation = self.hits_generation.wrapping_add(1);
        }
        if result.direction_changed {
            self.skill_breakdown_index
                .promote_unknown_direction(hit_damage);
            if let Some(promoted_hit) = promoted_hit {
                promote_unknown_direction_totals(&mut self.stats, promoted_hit);
            }
        }
    }

    pub fn duration_with_time_stop(&self, subtract_time_stop: bool) -> f64 {
        match (self.started_at, self.ended_at) {
            (Some(start), Some(end)) => {
                let raw = end - start;
                if subtract_time_stop {
                    (raw - self.time_stop.frozen_between(start, end)).max(0.001)
                } else {
                    raw.max(0.001)
                }
            }
            _ => 0.0,
        }
    }

    pub fn dps_with_time_stop(&self, subtract_time_stop: bool) -> f64 {
        self.total_damage / self.duration_with_time_stop(subtract_time_stop).max(1.0)
    }

    pub fn compact_timeline(
        &self,
        bucket_seconds: f64,
        max_buckets: usize,
    ) -> Option<CompactTimelineSeries> {
        self.compact_timeline
            .project(bucket_seconds, max_buckets, 0, 0)
    }

    pub const fn bounded_timeline_character_count(&self) -> BoundedTimelineCharacterCount {
        self.compact_timeline.bounded_character_count()
    }

    pub fn damage_attribution_summary(&self) -> DamageAttributionSummary {
        summarize_damage_attribution(
            self.total_damage,
            self.max_hp_reduction,
            self.stats.values(),
        )
    }

    pub fn skill_breakdown(&self) -> SkillBreakdown {
        self.skill_breakdown_index.project()
    }

    pub fn indexed_combat_details(
        &self,
        character_id: Option<u32>,
        filter: &IndexedCombatDetailFilter,
        skill: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> IndexedCombatDetailPage<'_> {
        self.combat_detail_index
            .query(&self.hits, character_id, filter, skill, offset, limit)
    }

    pub fn character_duration_with_time_stop(
        &self,
        row: &CharacterStats,
        subtract_time_stop: bool,
    ) -> f64 {
        character_duration_after_time_stop(row, &self.time_stop, subtract_time_stop)
    }

    pub fn character_dps_with_time_stop(
        &self,
        row: &CharacterStats,
        subtract_time_stop: bool,
    ) -> f64 {
        row.damage
            / self
                .character_duration_with_time_stop(row, subtract_time_stop)
                .max(1.0)
    }

    pub fn apply_time_stop_event(&mut self, event: &TimeStopEvent) {
        self.time_stop.apply_event(event);
        self.sync_clock_with_time_stops();
    }

    fn sync_clock_with_time_stops(&mut self) {
        sync_combat_clock_with_time_stops(self.started_at, &mut self.ended_at, &self.time_stop);
    }

    #[allow(dead_code)]
    pub fn time_stop_intervals_between(
        &self,
        start: f64,
        end: f64,
    ) -> Vec<TimelineTimeStopInterval> {
        relative_time_stop_intervals(&self.time_stop, start, end)
    }

    pub fn timeline(&self, bucket_seconds: f64, subtract_time_stop: bool) -> TimelineSeries {
        self.timeline_bounded(
            bucket_seconds,
            subtract_time_stop,
            DEFAULT_MAX_TIMELINE_BUCKETS,
            DEFAULT_MAX_TIMELINE_ROLES_PER_BUCKET,
            DEFAULT_MAX_TIMELINE_CHARACTERS,
        )
    }

    pub fn timeline_bounded(
        &self,
        bucket_seconds: f64,
        subtract_time_stop: bool,
        max_buckets: usize,
        max_roles_per_bucket: usize,
        max_characters: usize,
    ) -> TimelineSeries {
        summarize_indexed_timeline(
            &self.compact_timeline,
            &self.time_stop,
            self.started_at,
            self.ended_at,
            Vec::new(),
            TimelineAggregationOptions {
                bucket_seconds,
                subtract_time_stop,
                max_buckets,
                max_roles_per_bucket,
                max_characters,
            },
        )
    }
}

#[derive(Clone, Debug, Default)]
pub struct AbyssRunState {
    pub floor: Option<u32>,
    pub active_half: Option<AbyssHalf>,
    pub pending_restart_at: Option<f64>,
    pub pending_restart_half: Option<AbyssHalf>,
    pub last_half_switch_at: Option<f64>,
    pub last_half_switch_from: Option<AbyssHalf>,
    pub first_half_at: Option<f64>,
    pub second_half_at: Option<f64>,
    pub first_half: PartyCombatState,
    pub second_half: PartyCombatState,
    pub success_at: Option<f64>,
    pub exited_at: Option<f64>,
    pub event_count: u64,
    character_halves: HashMap<u32, AbyssHalf>,
}

impl AbyssRunState {
    pub fn is_active(&self) -> bool {
        self.floor.is_some()
            || !self.first_half.hits.is_empty()
            || !self.second_half.hits.is_empty()
            || self.success_at.is_some()
    }

    pub fn half(&self, half: AbyssHalf) -> &PartyCombatState {
        match half {
            AbyssHalf::First => &self.first_half,
            AbyssHalf::Second => &self.second_half,
        }
    }

    fn half_mut(&mut self, half: AbyssHalf) -> &mut PartyCombatState {
        match half {
            AbyssHalf::First => &mut self.first_half,
            AbyssHalf::Second => &mut self.second_half,
        }
    }

    fn clear_restarted_half(&mut self, half: AbyssHalf, timestamp: f64) {
        *self.half_mut(half) = PartyCombatState::default();
        self.character_halves
            .retain(|_, character_half| *character_half != half);
        self.success_at = None;
        self.exited_at = None;
        match half {
            AbyssHalf::First => self.first_half_at = Some(timestamp),
            AbyssHalf::Second => self.second_half_at = Some(timestamp),
        }
    }

    fn clear_restarted_floor(&mut self) {
        self.first_half = PartyCombatState::default();
        self.second_half = PartyCombatState::default();
        self.first_half_at = None;
        self.second_half_at = None;
        self.success_at = None;
        self.exited_at = None;
        self.character_halves.clear();
    }

    pub fn apply_event(&mut self, event: AbyssEvent) {
        self.event_count = self.event_count.saturating_add(1);
        match event {
            AbyssEvent::RestartDetected { timestamp } => {
                if let Some(half) = self.active_half {
                    self.clear_restarted_half(half, timestamp);
                    self.pending_restart_at = Some(timestamp);
                    self.pending_restart_half = Some(half);
                } else {
                    self.pending_restart_at = Some(timestamp);
                    self.pending_restart_half = None;
                }
                self.last_half_switch_at = None;
                self.last_half_switch_from = None;
            }
            AbyssEvent::Stage {
                timestamp,
                cycle: _,
                floor,
                half,
                allow_late_backfill: _,
            } => {
                let floor_changed = self
                    .floor
                    .zip(floor)
                    .is_some_and(|(current, next)| current != next);
                if floor_changed {
                    self.clear_restarted_floor();
                    self.active_half = None;
                    self.pending_restart_at = None;
                    self.pending_restart_half = None;
                    self.last_half_switch_at = None;
                    self.last_half_switch_from = None;
                }
                if floor.is_some() {
                    self.floor = floor;
                }
                if self.active_half.is_some_and(|active| active != half) {
                    self.last_half_switch_at = Some(timestamp);
                    self.last_half_switch_from = self.active_half;
                }
                if !floor_changed && let Some(restart_at) = self.pending_restart_at.take() {
                    let restarted_half = self.pending_restart_half.take();
                    if restarted_half.is_some_and(|previous_half| {
                        previous_half != half
                            && timestamp >= restart_at
                            && timestamp - restart_at <= ABYSS_RESTART_STAGE_WINDOW_SECONDS
                    }) {
                        self.clear_restarted_floor();
                    } else if restarted_half.is_none() {
                        self.clear_restarted_half(half, restart_at);
                    }
                }
                self.active_half = Some(half);
                match half {
                    AbyssHalf::First => {
                        self.first_half_at = Some(
                            self.first_half_at
                                .map_or(timestamp, |value| value.min(timestamp)),
                        );
                    }
                    AbyssHalf::Second => {
                        self.second_half_at = Some(
                            self.second_half_at
                                .map_or(timestamp, |value| value.min(timestamp)),
                        );
                    }
                }
            }
            AbyssEvent::Success { timestamp } => self.success_at = Some(timestamp),
            AbyssEvent::Exit { timestamp } => {
                self.exited_at = Some(timestamp);
                self.active_half = None;
                self.pending_restart_at = None;
                self.pending_restart_half = None;
                self.last_half_switch_at = None;
                self.last_half_switch_from = None;
            }
        }
    }

    pub fn push_hit(&mut self, hit: Hit) -> Option<AbyssHalf> {
        let active_half = self.active_half?;
        let half = if hit.char_known {
            if let Some(half) = self.character_halves.get(&hit.char_id) {
                *half
            } else if self.character_halves.len() < MAX_COMBAT_STAT_ROWS {
                self.character_halves.insert(hit.char_id, active_half);
                active_half
            } else {
                // The authoritative per-half hit remains complete. Only the
                // hostile-id routing cache is capped; excess ids follow the
                // currently observed half rather than allocating forever.
                active_half
            }
        } else {
            active_half
        };
        self.half_mut(half).push_hit(hit);
        Some(half)
    }

    pub fn apply_time_stop_event(&mut self, event: &TimeStopEvent) {
        let timestamp = match event {
            TimeStopEvent::GamePauseStarted { timestamp, .. }
            | TimeStopEvent::GamePauseEnded { timestamp, .. } => *timestamp,
        };
        let half = if self
            .second_half_at
            .is_some_and(|started_at| timestamp >= started_at)
        {
            AbyssHalf::Second
        } else if self
            .first_half_at
            .is_some_and(|started_at| timestamp >= started_at)
        {
            AbyssHalf::First
        } else {
            let Some(active_half) = self.active_half else {
                return;
            };
            active_half
        };
        self.half_mut(half).apply_time_stop_event(event);
    }

    pub fn timeline_markers_for_half(
        &self,
        half: AbyssHalf,
        start: f64,
        end: f64,
    ) -> Vec<TimelineMarker> {
        let mut markers = Vec::new();
        let (timestamp, label) = match half {
            AbyssHalf::First => (self.first_half_at, "Ascending Line"),
            AbyssHalf::Second => (self.second_half_at, "Descending Line"),
        };
        push_timeline_marker(
            &mut markers,
            timestamp,
            start,
            end,
            label,
            TimelineMarkerKind::HalfStart,
        );
        push_timeline_marker(
            &mut markers,
            self.success_at,
            start,
            end,
            "Cleared",
            TimelineMarkerKind::Clear,
        );
        push_timeline_marker(
            &mut markers,
            self.exited_at,
            start,
            end,
            "Left",
            TimelineMarkerKind::Exit,
        );
        sort_timeline_markers(&mut markers);
        markers
    }

    fn timeline_markers_between(&self, start: f64, end: f64) -> Vec<TimelineMarker> {
        let mut markers = Vec::new();
        push_timeline_marker(
            &mut markers,
            self.first_half_at,
            start,
            end,
            "Ascending Line",
            TimelineMarkerKind::HalfStart,
        );
        push_timeline_marker(
            &mut markers,
            self.second_half_at,
            start,
            end,
            "Descending Line",
            TimelineMarkerKind::HalfStart,
        );
        push_timeline_marker(
            &mut markers,
            self.success_at,
            start,
            end,
            "Cleared",
            TimelineMarkerKind::Clear,
        );
        push_timeline_marker(
            &mut markers,
            self.exited_at,
            start,
            end,
            "Left",
            TimelineMarkerKind::Exit,
        );
        sort_timeline_markers(&mut markers);
        markers
    }
}

const ENEMY_TELEMETRY_MOD_ID: &str = "enemy-telemetry";
const ENEMY_TELEMETRY_BACKFILL_HITS: usize = 32;
const ENEMY_TELEMETRY_MAX_HIT_TARGETS: usize = 128;
const MAX_SERVER_TARGET_DAMAGE_LIMITS: usize = 128;
const SERVER_DAMAGE_RECONCILIATION_WINDOW_SECONDS: f64 = 2.0;
const ENEMY_TELEMETRY_HIT_TARGET_WINDOW_SECONDS: f64 = 0.35;
const ENEMY_TELEMETRY_INSTANCE: &str = "target_name_resolution=enemy_telemetry_instance";
const ENEMY_TELEMETRY_HIT_INSTANCE: &str = "target_name_resolution=enemy_telemetry_hit_instance";
const FILETIME_UNIX_EPOCH_100NS: u64 = 116_444_736_000_000_000;
const FILETIME_TICKS_PER_SECOND: f64 = 10_000_000.0;

#[derive(Clone, Default)]
struct EnemyTelemetryTracker {
    hit_targets: VecDeque<EnemyHitTargetObservation>,
}

#[derive(Clone)]
struct EnemyHitTargetObservation {
    sequence: u64,
    target: u64,
    config_hash: u64,
    identity: Option<EnemyIdentity>,
    level: u64,
    observed_at: f64,
}

impl EnemyTelemetryTracker {
    fn apply_event(&mut self, event: &ModScriptEvent) -> Option<EnemyHitTargetObservation> {
        if event.mod_id != ENEMY_TELEMETRY_MOD_ID {
            return None;
        }
        let timestamp = filetime_100ns_to_unix_seconds(event.timestamp_100ns)?;
        match (event.phase, event.name.as_str(), event.values.as_slice()) {
            (
                ModScriptEventPhase::Postprocess,
                "enemy.hit_target",
                [target, config_hash, level],
            ) if *target != 0 && *config_hash != 0 => {
                if self.hit_targets.len() == ENEMY_TELEMETRY_MAX_HIT_TARGETS {
                    self.hit_targets.pop_front();
                }
                let observation = EnemyHitTargetObservation {
                    sequence: event.sequence,
                    target: *target,
                    config_hash: *config_hash,
                    identity: event
                        .enemy_identity
                        .as_ref()
                        .filter(|identity| identity.config_hash == *config_hash)
                        .cloned(),
                    level: *level,
                    observed_at: timestamp,
                };
                self.hit_targets.push_back(observation.clone());
                Some(observation)
            }
            _ => None,
        }
    }

    fn take_hit_target_for_hit(&mut self, hit: &Hit) -> Option<EnemyHitTargetObservation> {
        while self.hit_targets.front().is_some_and(|target| {
            target.observed_at + ENEMY_TELEMETRY_HIT_TARGET_WINDOW_SECONDS < hit.timestamp
        }) {
            self.hit_targets.pop_front();
        }
        let index = self.hit_targets.iter().position(|target| {
            hit_accepts_enemy_hit_target(hit, target)
                && (target.observed_at - hit.timestamp).abs()
                    <= ENEMY_TELEMETRY_HIT_TARGET_WINDOW_SECONDS
        })?;
        self.hit_targets.remove(index)
    }

    fn consume_hit_target(&mut self, sequence: u64) {
        if let Some(index) = self
            .hit_targets
            .iter()
            .position(|target| target.sequence == sequence)
        {
            self.hit_targets.remove(index);
        }
    }
}

fn filetime_100ns_to_unix_seconds(timestamp_100ns: u64) -> Option<f64> {
    timestamp_100ns
        .checked_sub(FILETIME_UNIX_EPOCH_100NS)
        .map(|ticks| ticks as f64 / FILETIME_TICKS_PER_SECOND)
}

fn hit_accepts_enemy_telemetry(hit: &Hit) -> bool {
    !hit.direction.is_incoming()
}

fn hit_has_exact_enemy_target(hit: &Hit) -> bool {
    hit.target_context
        .iter()
        .any(|context| context == ENEMY_TELEMETRY_HIT_INSTANCE)
}

fn hit_accepts_enemy_hit_target(hit: &Hit, target: &EnemyHitTargetObservation) -> bool {
    hit_accepts_enemy_telemetry(hit)
        && !hit_has_exact_enemy_target(hit)
        && hit.target_id.as_deref().is_none_or(|target_id| {
            target_id == format!("enemy:{:016x}", target.config_hash)
                || hit
                    .target_context
                    .iter()
                    .any(|context| context == ENEMY_TELEMETRY_INSTANCE)
        })
}

fn project_enemy_hit_target(
    hit: &mut Hit,
    target: &EnemyHitTargetObservation,
) -> EnemyTargetProjectionResult {
    let direction_changed = hit.direction.is_unknown();
    let target_id = format!("enemy-instance:{:016x}", target.target);
    let target_instance = format!("enemy_target_instance={:016x}", target.target);
    let identity_changed = target.identity.as_ref().is_some_and(|identity| {
        hit.target_name.as_deref() != Some(&identity.name_zh)
            || hit.target_name_en.as_deref() != Some(&identity.name_en)
            || hit.target_name_ja.as_deref() != Some(&identity.name_ja)
            || hit.target_monster_id.as_deref() != Some(&identity.monster_id)
    });
    let changed = direction_changed
        || hit.target_id.as_deref() != Some(&target_id)
        || identity_changed
        || !hit_has_exact_enemy_target(hit)
        || !hit
            .target_context
            .iter()
            .any(|context| context == &target_instance);

    if direction_changed {
        hit.direction = HitDirection::Outgoing;
    }
    hit.target_id = Some(target_id);
    if let Some(identity) = &target.identity {
        hit.target_name = Some(identity.name_zh.clone());
        hit.target_name_en = Some(identity.name_en.clone());
        hit.target_name_ja = Some(identity.name_ja.clone());
        hit.target_monster_id = Some(identity.monster_id.clone());
    }
    hit.target_context.retain(|context| {
        context != ENEMY_TELEMETRY_INSTANCE
            && context != ENEMY_TELEMETRY_HIT_INSTANCE
            && !context.starts_with("enemy_target_instance=")
            && !context.starts_with("enemy_config_id=")
            && !context.starts_with("enemy_level=")
    });
    hit.target_context
        .push(ENEMY_TELEMETRY_HIT_INSTANCE.to_owned());
    if let Some(identity) = &target.identity {
        hit.target_context
            .push(format!("enemy_config_id={}", identity.config_id));
    }
    if target.level > 0 {
        hit.target_context
            .push(format!("enemy_level={}", target.level));
    }
    hit.target_context.push(target_instance);

    EnemyTargetProjectionResult {
        changed,
        direction_changed,
    }
}

#[derive(Clone, Copy)]
struct EnemyHitKey {
    timestamp: u64,
    char_id: u32,
    byte_offset: usize,
    bit_shift: u8,
}

impl EnemyHitKey {
    fn from_hit(hit: &Hit) -> Self {
        Self {
            timestamp: hit.timestamp.to_bits(),
            char_id: hit.char_id,
            byte_offset: hit.byte_offset,
            bit_shift: hit.bit_shift,
        }
    }

    fn matches(self, hit: &Hit) -> bool {
        hit.timestamp.to_bits() == self.timestamp
            && hit.char_id == self.char_id
            && hit.byte_offset == self.byte_offset
            && hit.bit_shift == self.bit_shift
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct EnemyTargetProjectionResult {
    changed: bool,
    direction_changed: bool,
}

/// Declares whether an applied [`ModScriptEvent`] mutated any user-visible
/// combat projection. Reducers must key frontend revision bumps off this
/// outcome instead of guessing from the event kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModScriptApplyOutcome {
    #[default]
    Unchanged,
    ProjectionChanged,
}

fn outcome_of_projection(result: EnemyTargetProjectionResult) -> ModScriptApplyOutcome {
    if result.changed || result.direction_changed {
        ModScriptApplyOutcome::ProjectionChanged
    } else {
        ModScriptApplyOutcome::Unchanged
    }
}

fn backfill_enemy_hit_target(
    hits: &mut VecDeque<Hit>,
    target: &EnemyHitTargetObservation,
) -> Option<(EnemyHitKey, EnemyTargetProjectionResult, f64, usize, Hit)> {
    let first = hits.len().saturating_sub(ENEMY_TELEMETRY_BACKFILL_HITS);
    let (offset, hit) = hits.iter_mut().skip(first).enumerate().find(|(_, hit)| {
        hit_accepts_enemy_hit_target(hit, target)
            && (target.observed_at - hit.timestamp).abs()
                <= ENEMY_TELEMETRY_HIT_TARGET_WINDOW_SECONDS
    })?;
    let position = first.saturating_add(offset);
    let key = EnemyHitKey::from_hit(hit);
    let before = hit.clone();
    let result = project_enemy_hit_target(hit, target);
    Some((key, result, hit.total_damage(), position, before))
}

fn apply_enemy_hit_target_to_key(
    hits: &mut VecDeque<Hit>,
    key: EnemyHitKey,
    target: &EnemyHitTargetObservation,
) -> (EnemyTargetProjectionResult, f64, Option<(usize, Hit)>) {
    let hits_len = hits.len();
    match hits
        .iter_mut()
        .rev()
        .take(ENEMY_TELEMETRY_BACKFILL_HITS)
        .enumerate()
        .find(|(_, hit)| key.matches(hit))
    {
        Some((reverse_offset, hit)) => {
            let position = hits_len.saturating_sub(reverse_offset.saturating_add(1));
            let before = hit.clone();
            let result = project_enemy_hit_target(hit, target);
            (result, hit.total_damage(), Some((position, before)))
        }
        None => (EnemyTargetProjectionResult::default(), 0.0, None),
    }
}

#[derive(Clone, Default)]
pub struct CombatState {
    pub hits: VecDeque<Hit>,
    /// Exact per-global-hit Abyss membership used by cursor pagination. This
    /// sidecar stays position-aligned with `hits`; unlike the old read path it
    /// never reconstructs membership by hashing both complete half histories.
    global_hit_abyss_halves: VecDeque<Option<AbyssHalf>>,
    pub hits_generation: u64,
    pub packets: VecDeque<PacketDebug>,
    pub packets_generation: u64,
    pub packet_count: usize,
    pub packets_with_hits: usize,
    pub stats: HashMap<u32, CharacterStats>,
    pub started_at: Option<f64>,
    pub ended_at: Option<f64>,
    pub total_damage: f64,
    pub total_damage_taken: f64,
    pub max_hp_reduction: f64,
    pub abyss: AbyssRunState,
    pub damage_correction_count: u64,
    pub unattributed_server_damage_events: u64,
    pub unattributed_server_damage: f64,
    pub empty_curtain: Vec<EmptyCurtainItem>,
    pub empty_curtain_characters: Vec<EmptyCurtainCharacter>,
    pub empty_curtain_generation: u64,
    pub empty_curtain_characters_generation: u64,
    pub time_stop_events: Vec<TimeStopEvent>,
    pub combat_clock_health: CombatClockRuntimeHealth,
    compact_timeline: CompactTimelineIndex,
    skill_breakdown_index: SkillBreakdownIndex,
    combat_detail_index: CombatDetailIndex,
    time_stop: TimeStopTracker,
    enemy_telemetry: EnemyTelemetryTracker,
    packet_debug_bytes: usize,
    /// Bounded mutation index for delayed follow-up/correction events. This is
    /// runtime-only state and is rebuilt naturally by the import/replay push
    /// path; it is never part of a persisted or cross-boundary contract.
    recent_hit_records: VecDeque<RecentHitRecord>,
    server_target_damage_limits: HashMap<String, (Option<AbyssHalf>, Hit)>,
}

impl CombatState {
    /// Produces the single low-frequency snapshot needed by History archive
    /// preparation. It preserves every hit and every aggregate consumed by
    /// `session_summary`, but deliberately excludes debug packets, inventory,
    /// mutation caches, enemy telemetry, and the arena-backed timeline index.
    /// Those runtime-only payloads can be large and are neither persisted nor
    /// consulted by History serialization.
    pub fn clone_for_history_archive(&self) -> Self {
        Self {
            hits: self.hits.clone(),
            global_hit_abyss_halves: self.global_hit_abyss_halves.clone(),
            hits_generation: self.hits_generation,
            packets: VecDeque::new(),
            packets_generation: 0,
            packet_count: self.packet_count,
            packets_with_hits: self.packets_with_hits,
            stats: self.stats.clone(),
            started_at: self.started_at,
            ended_at: self.ended_at,
            total_damage: self.total_damage,
            total_damage_taken: self.total_damage_taken,
            max_hp_reduction: self.max_hp_reduction,
            abyss: self.abyss.clone(),
            damage_correction_count: self.damage_correction_count,
            unattributed_server_damage_events: self.unattributed_server_damage_events,
            unattributed_server_damage: self.unattributed_server_damage,
            empty_curtain: Vec::new(),
            empty_curtain_characters: Vec::new(),
            empty_curtain_generation: 0,
            empty_curtain_characters_generation: 0,
            time_stop_events: self.time_stop_events.clone(),
            combat_clock_health: self.combat_clock_health,
            compact_timeline: CompactTimelineIndex::default(),
            skill_breakdown_index: self.skill_breakdown_index.clone(),
            combat_detail_index: CombatDetailIndex::default(),
            time_stop: self.time_stop.clone(),
            enemy_telemetry: EnemyTelemetryTracker::default(),
            packet_debug_bytes: 0,
            recent_hit_records: VecDeque::new(),
            server_target_damage_limits: HashMap::new(),
        }
    }

    pub fn reconcile_server_target_damage(&mut self, marker: Hit) -> bool {
        if !marker.is_server_damage_reconciliation()
            || !marker.target_hp_before.is_finite()
            || marker.target_hp_before <= 0.0
        {
            return false;
        }
        let Some(target_id) = marker.target_id.clone() else {
            return false;
        };
        let target_half = self
            .hits
            .iter()
            .zip(&self.global_hit_abyss_halves)
            .rev()
            .find(|(hit, _)| {
                hit.direction.is_outgoing() && hit.target_id.as_deref() == Some(target_id.as_str())
            })
            .map(|(_, half)| *half)
            .unwrap_or(self.abyss.active_half);
        if !self.server_target_damage_limits.contains_key(&target_id)
            && self.server_target_damage_limits.len() >= MAX_SERVER_TARGET_DAMAGE_LIMITS
            && let Some(oldest) = self
                .server_target_damage_limits
                .iter()
                .min_by(|(left_id, (_, left)), (right_id, (_, right))| {
                    left.timestamp
                        .total_cmp(&right.timestamp)
                        .then_with(|| left_id.cmp(right_id))
                })
                .map(|(target_id, _)| target_id.clone())
        {
            self.server_target_damage_limits.remove(&oldest);
        }
        self.server_target_damage_limits
            .insert(target_id.clone(), (target_half, marker.clone()));
        self.reconcile_server_target_limit(&target_id, target_half, marker)
    }

    pub fn reconcile_known_server_target_limits(&mut self, source_timestamp: f64) -> bool {
        let limits = self
            .server_target_damage_limits
            .iter()
            .filter(|(_, (_, marker))| {
                marker.timestamp >= source_timestamp
                    && marker.timestamp - source_timestamp
                        <= SERVER_DAMAGE_RECONCILIATION_WINDOW_SECONDS
            })
            .map(|(target_id, (half, marker))| (target_id.clone(), *half, marker.clone()))
            .collect::<Vec<_>>();
        limits
            .into_iter()
            .fold(false, |changed, (target_id, half, marker)| {
                self.reconcile_server_target_limit(&target_id, half, marker) || changed
            })
    }

    fn reconcile_server_target_limit(
        &mut self,
        target_id: &str,
        target_half: Option<AbyssHalf>,
        mut marker: Hit,
    ) -> bool {
        let authoritative_damage = marker.target_hp_before.min(marker.target_max_hp);
        let represented_damage = self
            .hits
            .iter()
            .zip(&self.global_hit_abyss_halves)
            .filter(|(hit, half)| {
                **half == target_half
                    && hit.direction.is_outgoing()
                    && hit.target_id.as_deref() == Some(target_id)
            })
            .map(|(hit, _)| (hit.total_damage() - hit.overkill_damage()).max(0.0))
            .sum::<f64>();
        let residual = authoritative_damage - represented_damage;
        if residual >= 2.0 {
            marker.damage = residual;
            marker.target_hp_before = residual;
            marker.reconciled_overkill_damage = Some(0.0);
            self.push_hit_at_recorded_half(marker, target_half);
            return true;
        }
        if residual > -2.0 {
            return false;
        }

        let mut excess = -residual;
        let sources = self
            .hits
            .iter()
            .zip(&self.global_hit_abyss_halves)
            .rev()
            .filter(|(hit, half)| {
                **half == target_half
                    && hit.direction.is_outgoing()
                    && hit.target_id.as_deref() == Some(target_id)
                    && hit.damage - hit.overkill_damage() >= 2.0
            })
            .map(|(hit, _)| hit.clone())
            .collect::<Vec<_>>();
        let mut changed = false;
        for source in sources {
            if excess < 2.0 {
                break;
            }
            let effective = (source.damage - source.overkill_damage()).max(0.0);
            let adjustment = excess.min(effective);
            let correction = HitDamageCorrection {
                source_timestamp: source.timestamp,
                source_char_id: source.char_id,
                source_damage: source.damage,
                source_target_hp_before: source.target_hp_before,
                source_target_hp_after: source.target_hp_after,
                source_target_max_hp: source.target_max_hp,
                source_gameplay_effect_index: source.gameplay_effect_index,
                damage: source.damage,
                target_hp_before: source.target_hp_before,
                target_hp_after: source.target_hp_after,
                target_hp_percent: source.target_hp_percent,
                max_hp_reduction: None,
                reconciled_overkill_damage: Some(source.overkill_damage() + adjustment),
            };
            if self.apply_damage_correction(correction) {
                changed = true;
                excess -= adjustment;
            }
        }
        changed
    }

    pub fn push_hit(&mut self, mut hit: Hit) {
        if let Some(target) = self.enemy_telemetry.take_hit_target_for_hit(&hit) {
            project_enemy_hit_target(&mut hit, &target);
        }
        let abyss_half = self.abyss.push_hit(hit.clone());
        self.finish_push_hit(hit, abyss_half);
    }

    fn push_hit_at_recorded_half(&mut self, mut hit: Hit, abyss_half: Option<AbyssHalf>) {
        if let Some(target) = self.enemy_telemetry.take_hit_target_for_hit(&hit) {
            project_enemy_hit_target(&mut hit, &target);
        }
        if let Some(half) = abyss_half {
            self.abyss.half_mut(half).push_hit(hit.clone());
        }
        self.finish_push_hit(hit, abyss_half);
    }

    fn finish_push_hit(&mut self, hit: Hit, abyss_half: Option<AbyssHalf>) {
        let position = self.hits.len();
        update_combat_totals(
            &mut self.stats,
            &mut self.compact_timeline,
            &mut self.started_at,
            &mut self.ended_at,
            &mut self.total_damage,
            &mut self.total_damage_taken,
            &mut self.max_hp_reduction,
            &hit,
        );
        self.skill_breakdown_index.observe_hit(&hit);
        if let Some(stable_hit) = position
            .checked_sub(RECENT_HIT_MUTATION_WINDOW)
            .and_then(|position| self.hits.get(position))
        {
            self.combat_detail_index.promote_stable_hit(stable_hit);
        }
        self.combat_detail_index.observe_hit(position, &hit);
        remember_recent_hit(&mut self.recent_hit_records, &hit, abyss_half);
        self.hits.push_back(hit);
        reconcile_latest_overkill_interval(&mut self.hits);
        self.global_hit_abyss_halves.push_back(abyss_half);
        self.hits_generation = self.hits_generation.wrapping_add(1);
        self.sync_clock_with_time_stops();
    }

    /// Rehydrates an owned non-Abyss History combat without invoking the live
    /// push path once per hit. Derived indexes share one traversal and only the
    /// bounded recent-mutation locator window is rebuilt afterward.
    pub(crate) fn replace_global_hits_bulk(&mut self, hits: Vec<Hit>) {
        self.hits = hits.into();
        self.global_hit_abyss_halves =
            std::iter::repeat_n(None, self.hits.len()).collect::<VecDeque<_>>();
        self.hits_generation = u64::try_from(self.hits.len()).unwrap_or(u64::MAX);
        rebuild_all_combat_indexes(
            &self.hits,
            &mut self.stats,
            &mut self.compact_timeline,
            &mut self.skill_breakdown_index,
            &mut self.combat_detail_index,
            &mut self.started_at,
            &mut self.ended_at,
            &mut self.total_damage,
            &mut self.total_damage_taken,
            &mut self.max_hp_reduction,
        );
        self.recent_hit_records.clear();
        for hit in self
            .hits
            .iter()
            .skip(self.hits.len().saturating_sub(RECENT_HIT_MUTATION_WINDOW))
        {
            remember_recent_hit(&mut self.recent_hit_records, hit, None);
        }
        self.sync_clock_with_time_stops();
    }

    fn locate_recent_hit(
        &mut self,
        source: HitSourceIdentity,
    ) -> Option<(usize, HitLocator, Option<AbyssHalf>)> {
        if let Some(index) = self
            .recent_hit_records
            .iter()
            .rposition(|record| record.matches_source(source))
        {
            let record = self.recent_hit_records[index];
            return Some((index, record.locator, record.abyss_half));
        }

        // Recovery path for states constructed by older in-memory fixtures or
        // an internal index invariant failure. The scan is deliberately capped;
        // an unbounded miss must not stall the capture reducer under its hot
        // event/state locks.
        let locator = find_recent_hit_locator(&self.hits, source)?;
        let abyss_half = if recent_hits_contain_locator(&self.abyss.first_half.hits, locator) {
            Some(AbyssHalf::First)
        } else if recent_hits_contain_locator(&self.abyss.second_half.hits, locator) {
            Some(AbyssHalf::Second)
        } else {
            None
        };
        let mut record = RecentHitRecord::from_source(locator, source, abyss_half);
        if let Some(hit) = find_recent_hit(&self.hits, locator) {
            record.remember_source(HitSourceIdentity::from(hit));
        }
        push_recent_hit_record(&mut self.recent_hit_records, record);
        let index = self.recent_hit_records.len().saturating_sub(1);
        Some((index, locator, abyss_half))
    }

    pub fn apply_follow_up(&mut self, follow_up: HitFollowUp) -> bool {
        let source = HitSourceIdentity::from(&follow_up);
        let Some((record_index, locator, abyss_half)) = self.locate_recent_hit(source) else {
            return false;
        };
        let position = find_recent_hit_position(&self.hits, locator);
        let before = find_recent_hit(&self.hits, locator).cloned();
        let Some(mutation) = apply_follow_up_to_recent_hit(&mut self.hits, locator, &follow_up)
        else {
            return false;
        };
        if let (Some(before), Some(after)) = (before, find_recent_hit(&self.hits, locator)) {
            self.skill_breakdown_index.replace_hit(&before, after);
            if let Some(position) = position {
                self.combat_detail_index
                    .replace_hit(position, &before, after);
            }
        }

        self.hits_generation = self.hits_generation.wrapping_add(1);
        apply_combat_totals_delta(
            &self.hits,
            &mut self.stats,
            &mut self.compact_timeline,
            &mut self.started_at,
            &mut self.ended_at,
            &mut self.total_damage,
            &mut self.total_damage_taken,
            &mut self.max_hp_reduction,
            mutation,
        );
        self.recent_hit_records[record_index].remember_source(mutation.after_source);
        if let Some(half) = abyss_half {
            self.abyss
                .half_mut(half)
                .apply_follow_up_at(locator, &follow_up);
        }
        true
    }

    pub fn apply_damage_correction(&mut self, correction: HitDamageCorrection) -> bool {
        let source = HitSourceIdentity::from(&correction);
        let Some((record_index, locator, abyss_half)) = self.locate_recent_hit(source) else {
            return false;
        };
        let position = find_recent_hit_position(&self.hits, locator);
        let before = find_recent_hit(&self.hits, locator).cloned();
        let Some(mutation) =
            apply_damage_correction_to_recent_hit(&mut self.hits, locator, &correction)
        else {
            return false;
        };
        if let (Some(before), Some(after)) = (before, find_recent_hit(&self.hits, locator)) {
            self.skill_breakdown_index.replace_hit(&before, after);
            if let Some(position) = position {
                self.combat_detail_index
                    .replace_hit(position, &before, after);
            }
        }

        self.damage_correction_count = self.damage_correction_count.saturating_add(1);
        self.hits_generation = self.hits_generation.wrapping_add(1);
        apply_combat_totals_delta(
            &self.hits,
            &mut self.stats,
            &mut self.compact_timeline,
            &mut self.started_at,
            &mut self.ended_at,
            &mut self.total_damage,
            &mut self.total_damage_taken,
            &mut self.max_hp_reduction,
            mutation,
        );
        self.recent_hit_records[record_index].remember_source(mutation.after_source);
        if let Some(half) = abyss_half {
            self.abyss
                .half_mut(half)
                .apply_damage_correction_at(locator, &correction);
        }
        true
    }

    pub fn push_packet(&mut self, packet: PacketDebug) -> bool {
        let retained_bytes = packet.retained_heap_bytes();
        if retained_bytes > MAX_DEBUG_PACKET_BYTES {
            return false;
        }
        self.packet_debug_bytes = self.packet_debug_bytes.saturating_add(retained_bytes);
        self.packets.push_back(packet);
        self.packets_generation = self.packets_generation.wrapping_add(1);
        while self.packets.len() > MAX_DEBUG_PACKETS
            || self.packet_debug_bytes > MAX_DEBUG_PACKET_BYTES
        {
            let Some(removed) = self.packets.pop_front() else {
                self.packet_debug_bytes = 0;
                break;
            };
            self.packet_debug_bytes = self
                .packet_debug_bytes
                .saturating_sub(removed.retained_heap_bytes());
        }
        true
    }

    /// Replaces the equipment projection and reports whether observable state
    /// changed. Repeated snapshots are common during plugin polling and must
    /// not advance inventory revisions.
    pub fn replace_empty_curtain(&mut self, items: Vec<EmptyCurtainItem>) -> bool {
        if self.empty_curtain == items {
            return false;
        }
        self.empty_curtain = items;
        self.empty_curtain_generation = self.empty_curtain_generation.wrapping_add(1);
        true
    }

    pub fn observe_unattributed_server_damage(
        &mut self,
        observation: UnattributedServerDamage,
    ) -> bool {
        if !observation.timestamp.is_finite()
            || !observation.damage.is_finite()
            || observation.damage <= 0.0
        {
            return false;
        }
        self.unattributed_server_damage_events =
            self.unattributed_server_damage_events.saturating_add(1);
        self.unattributed_server_damage += observation.damage;
        true
    }

    /// Replaces the captured character mapping and reports whether observable
    /// state changed.
    pub fn replace_empty_curtain_characters(
        &mut self,
        characters: Vec<EmptyCurtainCharacter>,
    ) -> bool {
        if self.empty_curtain_characters == characters {
            return false;
        }
        self.empty_curtain_characters = characters;
        self.empty_curtain_characters_generation =
            self.empty_curtain_characters_generation.wrapping_add(1);
        true
    }

    fn apply_enemy_target_projection_result(
        &mut self,
        result: EnemyTargetProjectionResult,
        hit_damage: f64,
        promoted_hit: Option<HitAggregateContribution>,
    ) {
        if result.changed {
            self.hits_generation = self.hits_generation.wrapping_add(1);
        }
        if result.direction_changed {
            self.skill_breakdown_index
                .promote_unknown_direction(hit_damage);
            if let Some(promoted_hit) = promoted_hit {
                promote_unknown_direction_totals(&mut self.stats, promoted_hit);
            }
        }
    }

    pub fn apply_mod_script_event(&mut self, event: &ModScriptEvent) -> ModScriptApplyOutcome {
        let Some(target) = self.enemy_telemetry.apply_event(event) else {
            return ModScriptApplyOutcome::Unchanged;
        };
        let Some((key, result, hit_damage, position, before)) =
            backfill_enemy_hit_target(&mut self.hits, &target)
        else {
            return ModScriptApplyOutcome::Unchanged;
        };
        self.enemy_telemetry.consume_hit_target(target.sequence);
        let mut outcome = outcome_of_projection(result);
        let promoted_hit = result
            .direction_changed
            .then(|| self.hits.get(position).map(HitAggregateContribution::from))
            .flatten();
        if result.direction_changed
            && let Some(after) = self.hits.get(position)
        {
            self.combat_detail_index
                .replace_hit(position, &before, after);
        }
        self.apply_enemy_target_projection_result(result, hit_damage, promoted_hit);
        for party in [&mut self.abyss.first_half, &mut self.abyss.second_half] {
            let (result, hit_damage, mutation) =
                apply_enemy_hit_target_to_key(&mut party.hits, key, &target);
            let promoted_hit = if result.direction_changed {
                mutation
                    .as_ref()
                    .and_then(|(position, _)| party.hits.get(*position))
                    .map(HitAggregateContribution::from)
            } else {
                None
            };
            if result.direction_changed
                && let Some((position, before)) = mutation
                && let Some(after) = party.hits.get(position)
            {
                party
                    .combat_detail_index
                    .replace_hit(position, &before, after);
            }
            party.apply_enemy_target_projection_result(result, hit_damage, promoted_hit);
            if outcome_of_projection(result) == ModScriptApplyOutcome::ProjectionChanged {
                outcome = ModScriptApplyOutcome::ProjectionChanged;
            }
        }
        outcome
    }

    pub fn duration_with_time_stop(&self, subtract_time_stop: bool) -> f64 {
        match (self.started_at, self.ended_at) {
            (Some(start), Some(end)) => {
                let raw = end - start;
                if subtract_time_stop {
                    (raw - self.time_stop.frozen_between(start, end)).max(0.001)
                } else {
                    raw.max(0.001)
                }
            }
            _ => 0.0,
        }
    }

    pub fn active_elapsed_between(&self, start: f64, end: f64) -> f64 {
        (end - start - self.time_stop.frozen_between(start, end)).max(0.0)
    }

    pub fn dps_with_time_stop(&self, subtract_time_stop: bool) -> f64 {
        self.total_damage / self.duration_with_time_stop(subtract_time_stop).max(1.0)
    }

    pub fn compact_timeline(
        &self,
        bucket_seconds: f64,
        max_buckets: usize,
    ) -> Option<CompactTimelineSeries> {
        self.compact_timeline
            .project(bucket_seconds, max_buckets, 0, 0)
    }

    pub const fn bounded_timeline_character_count(&self) -> BoundedTimelineCharacterCount {
        self.compact_timeline.bounded_character_count()
    }

    /// Returns only the requested global-hit page. The iterator performs one
    /// O(1) indexed lookup per returned row and never walks the prefix before
    /// `offset`, so a deep CLI cursor does not hold the live-state lock for the
    /// age of the combat.
    pub fn global_hit_axis_page(
        &self,
        offset: usize,
        limit: usize,
    ) -> impl Iterator<Item = (&Hit, Option<AbyssHalf>)> {
        let start = offset.min(self.hits.len());
        let end = start.saturating_add(limit).min(self.hits.len());
        (start..end).filter_map(move |position| {
            self.hits.get(position).map(|hit| {
                #[cfg(all(test, feature = "cli"))]
                GLOBAL_HIT_AXIS_PAGE_VISITS.with(|visits| {
                    visits.set(visits.get().saturating_add(1));
                });
                let half = self
                    .global_hit_abyss_halves
                    .get(position)
                    .copied()
                    .flatten();
                (hit, half)
            })
        })
    }

    #[cfg(all(test, feature = "cli"))]
    pub(crate) fn install_large_axis_fixture(&mut self, hit: Hit, retained_hits: usize) {
        self.hits = std::iter::repeat_n(hit.clone(), retained_hits).collect();
        self.global_hit_abyss_halves =
            std::iter::repeat_n(None, retained_hits).collect::<VecDeque<_>>();
        self.hits_generation = u64::try_from(retained_hits).unwrap_or(u64::MAX);
        self.compact_timeline = CompactTimelineIndex::default();
        self.compact_timeline.observe_hit(&hit);
        self.started_at = Some(hit.timestamp);
        self.ended_at = Some(hit.timestamp);
    }

    pub fn damage_attribution_summary(&self) -> DamageAttributionSummary {
        summarize_damage_attribution(
            self.total_damage,
            self.max_hp_reduction,
            self.stats.values(),
        )
    }

    pub fn character_duration_with_time_stop(
        &self,
        row: &CharacterStats,
        subtract_time_stop: bool,
    ) -> f64 {
        character_duration_after_time_stop(row, &self.time_stop, subtract_time_stop)
    }

    pub fn character_dps_with_time_stop(
        &self,
        row: &CharacterStats,
        subtract_time_stop: bool,
    ) -> f64 {
        row.damage
            / self
                .character_duration_with_time_stop(row, subtract_time_stop)
                .max(1.0)
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn observe_packet(&mut self, observation: PacketObservation) {
        self.packet_count = self.packet_count.saturating_add(1);
        if observation.parsed_hits > 0 {
            self.packets_with_hits = self.packets_with_hits.saturating_add(1);
        }
    }

    pub fn take_battle_preserving_inventory(&mut self) -> CombatState {
        let mut detached = std::mem::take(self);
        self.empty_curtain = std::mem::take(&mut detached.empty_curtain);
        self.empty_curtain_characters = std::mem::take(&mut detached.empty_curtain_characters);
        self.empty_curtain_generation = detached.empty_curtain_generation;
        self.empty_curtain_characters_generation = detached.empty_curtain_characters_generation;
        // Provider connectivity belongs to the capture session, not one
        // battle. Keep it in the replacement state as well as the detached
        // archive; the provider only emits on transitions and would otherwise
        // remain falsely Unknown after every round cut.
        self.combat_clock_health = detached.combat_clock_health;
        detached
    }

    pub const fn effective_dps_time_basis(&self, requested: DpsTimeBasis) -> DpsTimeBasis {
        if requested.subtracts_time_stop()
            && self.combat_clock_health.supports_time_stop_adjustment()
        {
            DpsTimeBasis::SubtractTimeStop
        } else {
            DpsTimeBasis::WallClock
        }
    }

    pub fn clear_battle_preserving_inventory(&mut self) {
        let _ = self.take_battle_preserving_inventory();
    }

    pub fn apply_abyss_event(&mut self, event: AbyssEvent) {
        let first_half_had_hits = !self.abyss.first_half.hits.is_empty();
        let second_half_had_hits = !self.abyss.second_half.hits.is_empty();
        let late_detected_half = match &event {
            AbyssEvent::Stage {
                half,
                allow_late_backfill,
                ..
            } if *allow_late_backfill
                && self.abyss.active_half.is_none()
                && self.abyss.first_half.hits.is_empty()
                && self.abyss.second_half.hits.is_empty()
                && !self.hits.is_empty() =>
            {
                Some(*half)
            }
            _ => None,
        };
        self.abyss.apply_event(event);
        if first_half_had_hits && self.abyss.first_half.hits.is_empty() {
            self.clear_global_hit_abyss_half(AbyssHalf::First);
        }
        if second_half_had_hits && self.abyss.second_half.hits.is_empty() {
            self.clear_global_hit_abyss_half(AbyssHalf::Second);
        }
        if let Some(half) = late_detected_half {
            self.backfill_abyss_half_from_global(half);
        }
    }

    pub fn apply_time_stop_event(&mut self, event: TimeStopEvent) {
        self.time_stop.apply_event(&event);
        self.sync_clock_with_time_stops();
        self.abyss.apply_time_stop_event(&event);
        if self.time_stop_events.len() >= MAX_RETAINED_TIME_STOP_EVENTS {
            compact_time_stop_event_prefix(&mut self.time_stop_events);
        }
        self.time_stop_events.push(event);
    }

    pub fn set_combat_clock_health(&mut self, health: CombatClockRuntimeHealth) -> bool {
        if self.combat_clock_health == health {
            return false;
        }
        self.combat_clock_health = health;
        true
    }

    pub fn is_game_paused(&self) -> bool {
        self.time_stop.active_game_pause.is_some()
    }

    pub fn rebuild_global_from_abyss(&mut self) {
        let mut hits = self
            .abyss
            .first_half
            .hits
            .iter()
            .cloned()
            .map(|hit| (hit, AbyssHalf::First))
            .chain(
                self.abyss
                    .second_half
                    .hits
                    .iter()
                    .cloned()
                    .map(|hit| (hit, AbyssHalf::Second)),
            )
            .collect::<Vec<_>>();
        hits.sort_by(|left, right| {
            left.0
                .timestamp
                .total_cmp(&right.0.timestamp)
                .then_with(|| left.0.byte_offset.cmp(&right.0.byte_offset))
                .then_with(|| left.0.bit_shift.cmp(&right.0.bit_shift))
        });
        self.recent_hit_records.clear();
        for (hit, half) in hits.iter().rev().take(RECENT_HIT_MUTATION_WINDOW).rev() {
            remember_recent_hit(&mut self.recent_hit_records, hit, Some(*half));
        }
        let (global_hits, global_hit_abyss_halves): (VecDeque<_>, VecDeque<_>) = hits
            .into_iter()
            .map(|(hit, half)| (hit, Some(half)))
            .unzip();
        self.hits = global_hits;
        self.global_hit_abyss_halves = global_hit_abyss_halves;
        self.hits_generation = self.hits_generation.wrapping_add(1);
        rebuild_all_combat_indexes(
            &self.hits,
            &mut self.stats,
            &mut self.compact_timeline,
            &mut self.skill_breakdown_index,
            &mut self.combat_detail_index,
            &mut self.started_at,
            &mut self.ended_at,
            &mut self.total_damage,
            &mut self.total_damage_taken,
            &mut self.max_hp_reduction,
        );
        self.sync_clock_with_time_stops();
    }

    fn sync_clock_with_time_stops(&mut self) {
        sync_combat_clock_with_time_stops(self.started_at, &mut self.ended_at, &self.time_stop);
    }

    #[allow(dead_code)]
    pub fn time_stop_intervals_between(
        &self,
        start: f64,
        end: f64,
    ) -> Vec<TimelineTimeStopInterval> {
        relative_time_stop_intervals(&self.time_stop, start, end)
    }

    pub fn timeline(&self, bucket_seconds: f64, subtract_time_stop: bool) -> TimelineSeries {
        self.timeline_bounded(
            bucket_seconds,
            subtract_time_stop,
            DEFAULT_MAX_TIMELINE_BUCKETS,
            DEFAULT_MAX_TIMELINE_ROLES_PER_BUCKET,
            DEFAULT_MAX_TIMELINE_CHARACTERS,
        )
    }

    pub fn timeline_bounded(
        &self,
        bucket_seconds: f64,
        subtract_time_stop: bool,
        max_buckets: usize,
        max_roles_per_bucket: usize,
        max_characters: usize,
    ) -> TimelineSeries {
        let mut series = summarize_indexed_timeline(
            &self.compact_timeline,
            &self.time_stop,
            self.started_at,
            self.ended_at,
            Vec::new(),
            TimelineAggregationOptions {
                bucket_seconds,
                subtract_time_stop,
                max_buckets,
                max_roles_per_bucket,
                max_characters,
            },
        );
        if let (Some(start), Some(end)) = (series.start_timestamp, series.end_timestamp) {
            series.markers = self.abyss.timeline_markers_between(start, end);
        }
        series
    }

    pub fn skill_breakdown(&self, char_filter: Option<u32>) -> SkillBreakdown {
        match char_filter {
            None => self.skill_breakdown_index.project(),
            Some(_) => summarize_skill_breakdown(&self.hits, char_filter),
        }
    }

    pub fn indexed_combat_details(
        &self,
        character_id: Option<u32>,
        filter: &IndexedCombatDetailFilter,
        skill: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> IndexedCombatDetailPage<'_> {
        self.combat_detail_index
            .query(&self.hits, character_id, filter, skill, offset, limit)
    }

    #[cfg(feature = "desktop")]
    #[allow(dead_code)]
    pub(crate) fn capture_quality_scalars(&self) -> CaptureQualityScalars {
        let start = self.started_at.unwrap_or_default();
        let end = self.ended_at.unwrap_or_default();
        CaptureQualityScalars {
            hits_generation: self.hits_generation,
            packet_count: self.packet_count,
            packets_with_hits: self.packets_with_hits,
            hit_count: self.hits.len(),
            time_stop_event_count: self.time_stop.event_count,
            time_stop_interval_count: self.time_stop.interval_count_between(start, end),
            abyss_event_count: self.abyss.event_count,
            server_damage_corrections: self.damage_correction_count,
            unattributed_server_damage_events: self.unattributed_server_damage_events,
            unattributed_server_damage_bits: self.unattributed_server_damage.to_bits(),
        }
    }

    pub fn capture_quality_summary(&self, source: CaptureQualitySource) -> CaptureQualitySummary {
        let directions = self.skill_breakdown_index.directions;
        CaptureQualitySummary {
            source,
            packet_count: self.packet_count,
            packets_with_hits: self.packets_with_hits,
            hit_count: self.hits.len(),
            outgoing_hits: directions.outgoing_hits,
            outgoing_damage: directions.outgoing_damage,
            unknown_direction_hits: directions.unknown_hits,
            unknown_direction_damage: directions.unknown_damage,
            incoming_hits: directions.incoming_hits,
            incoming_damage: directions.incoming_damage,
            unknown_character_count: self.skill_breakdown_index.unknown_character_count(),
            unknown_character_hits: self.skill_breakdown_index.unknown_character_hits,
            unmapped_skill_rows: self.skill_breakdown_index.unmapped_skill_row_count(),
            unmapped_skill_hits: self.skill_breakdown_index.unmapped_skill_hits,
            unmapped_gameplay_effect_count: self
                .skill_breakdown_index
                .unmapped_gameplay_effect_count(),
            time_stop_event_count: self.time_stop.event_count,
            time_stop_interval_count: self
                .time_stop
                .intervals_between(
                    self.started_at.unwrap_or_default(),
                    self.ended_at.unwrap_or_default(),
                )
                .len(),
            abyss_event_count: self.abyss.event_count,
            server_damage_corrections: self.damage_correction_count,
            unattributed_server_damage_events: self.unattributed_server_damage_events,
            unattributed_server_damage: self.unattributed_server_damage,
        }
    }

    pub fn session_summary(
        &self,
        source: CaptureQualitySource,
        dps_time_mode: DpsTimeBasis,
        separate_reaction_damage: bool,
    ) -> Option<CombatSessionSummary> {
        if self.hits.is_empty() && self.stats.is_empty() && !self.abyss.is_active() {
            return None;
        }
        let subtract_time_stop = dps_time_mode.subtracts_time_stop();
        let duration = self.duration_with_time_stop(subtract_time_stop);
        let skills = summarize_session_skills(self.skill_breakdown(None).rows);
        let characters = self
            .stats
            .values()
            .map(|row| row.for_reaction_damage_policy(separate_reaction_damage))
            .collect::<Vec<_>>();
        Some(CombatSessionSummary {
            duration_seconds: duration,
            dps_time_mode,
            total_damage: self.total_damage,
            total_dps: self.dps_with_time_stop(subtract_time_stop),
            total_damage_taken: self.total_damage_taken,
            // `SkillBreakdownIndex` observes every retained hit direction,
            // including zero-damage hits. Reuse its exact O(1) counters rather
            // than scanning an unbounded combat log on the CLI's 250 ms battle
            // summary tick.
            total_hits: self
                .skill_breakdown_index
                .directions
                .outgoing_hits
                .saturating_add(self.skill_breakdown_index.directions.unknown_hits),
            reaction_damage_separated: separate_reaction_damage,
            damage_attribution: self.damage_attribution_summary(),
            characters: summarize_session_characters(characters.iter(), self.total_damage, |row| {
                self.character_dps_with_time_stop(row, subtract_time_stop)
            }),
            skills,
            abyss: summarize_session_abyss(
                &self.abyss,
                subtract_time_stop,
                separate_reaction_damage,
            ),
            quality: self.capture_quality_summary(source),
        })
    }

    fn backfill_abyss_half_from_global(&mut self, half: AbyssHalf) {
        if !self.abyss.half(half).hits.is_empty() {
            return;
        }
        for hit in self.hits.iter().cloned() {
            if hit.char_known
                && (self.abyss.character_halves.contains_key(&hit.char_id)
                    || self.abyss.character_halves.len() < MAX_COMBAT_STAT_ROWS)
            {
                self.abyss.character_halves.insert(hit.char_id, half);
            }
            self.abyss.half_mut(half).push_hit(hit);
        }
        for record in &mut self.recent_hit_records {
            record.abyss_half = Some(half);
        }
        self.global_hit_abyss_halves.clear();
        self.global_hit_abyss_halves
            .extend(std::iter::repeat_n(Some(half), self.hits.len()));
        self.abyss.half_mut(half).time_stop = self.time_stop.clone();
        self.abyss.half_mut(half).sync_clock_with_time_stops();
    }

    fn clear_global_hit_abyss_half(&mut self, half: AbyssHalf) {
        self.global_hit_abyss_halves.resize(self.hits.len(), None);
        self.global_hit_abyss_halves.truncate(self.hits.len());
        for retained_half in &mut self.global_hit_abyss_halves {
            if *retained_half == Some(half) {
                *retained_half = None;
            }
        }
        for record in &mut self.recent_hit_records {
            if record.abyss_half == Some(half) {
                record.abyss_half = None;
            }
        }
    }
}

fn summarize_session_abyss(
    abyss: &AbyssRunState,
    subtract_time_stop: bool,
    separate_reaction_damage: bool,
) -> CombatSessionAbyssSummary {
    CombatSessionAbyssSummary {
        detected: abyss.is_active(),
        floor: abyss.floor,
        active_half: abyss.active_half,
        success: abyss.success_at.is_some(),
        first_half: summarize_session_abyss_half(
            AbyssHalf::First,
            &abyss.first_half,
            subtract_time_stop,
            separate_reaction_damage,
        ),
        second_half: summarize_session_abyss_half(
            AbyssHalf::Second,
            &abyss.second_half,
            subtract_time_stop,
            separate_reaction_damage,
        ),
    }
}

fn summarize_session_abyss_half(
    half: AbyssHalf,
    party: &PartyCombatState,
    subtract_time_stop: bool,
    separate_reaction_damage: bool,
) -> Option<CombatSessionAbyssHalfSummary> {
    if party.hits.is_empty() && party.stats.is_empty() {
        return None;
    }
    let characters = party
        .stats
        .values()
        .map(|row| row.for_reaction_damage_policy(separate_reaction_damage))
        .collect::<Vec<_>>();
    Some(CombatSessionAbyssHalfSummary {
        half,
        duration_seconds: party.duration_with_time_stop(subtract_time_stop),
        total_damage: party.total_damage,
        total_dps: party.dps_with_time_stop(subtract_time_stop),
        damage_attribution: party.damage_attribution_summary(),
        characters: summarize_session_characters(characters.iter(), party.total_damage, |row| {
            party.character_dps_with_time_stop(row, subtract_time_stop)
        }),
        skills: summarize_session_skills(party.skill_breakdown().rows),
    })
}

fn summarize_session_characters<'a>(
    rows: impl IntoIterator<Item = &'a CharacterStats>,
    total_damage: f64,
    dps_for_row: impl Fn(&CharacterStats) -> f64,
) -> Vec<CombatSessionCharacterSummary> {
    let mut rows = rows
        .into_iter()
        .filter(|row| row.damage > 0.0 || row.damage_taken > 0.0 || row.hits > 0)
        .map(|row| CombatSessionCharacterSummary {
            char_id: row.char_id,
            name: row.name.clone(),
            hits: row.hits,
            damage: row.damage,
            dps: dps_for_row(row),
            damage_share_percent: if total_damage > 0.0 {
                row.damage / total_damage * 100.0
            } else {
                0.0
            },
            hits_taken: row.hits_taken,
            damage_taken: row.damage_taken,
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .damage
            .total_cmp(&left.damage)
            .then_with(|| left.char_id.cmp(&right.char_id))
    });
    rows
}

fn summarize_session_skills(rows: Vec<SkillBreakdownRow>) -> Vec<CombatSessionSkillSummary> {
    let total_damage = rows.iter().map(|row| row.damage).sum::<f64>();
    rows.into_iter()
        .map(|row| CombatSessionSkillSummary {
            char_id: row.char_id,
            char_name: row.char_name,
            name: row.name,
            category: row.category,
            ability_name: row.ability_name,
            gameplay_effect_name: row.gameplay_effect_name,
            damage_name: row.damage_name,
            hits: row.hits,
            damage: row.damage,
            damage_share_percent: if total_damage > 0.0 {
                row.damage / total_damage * 100.0
            } else {
                0.0
            },
            is_follow_up: row.is_follow_up,
        })
        .collect()
}

fn character_duration_after_time_stop(
    row: &CharacterStats,
    time_stop: &TimeStopTracker,
    subtract_time_stop: bool,
) -> f64 {
    let raw = row.duration();
    if raw <= 0.0 {
        return 0.0;
    }
    if !subtract_time_stop {
        return raw;
    }
    (raw - time_stop.frozen_between(row.first_hit, row.last_hit)).max(0.001)
}

fn sync_combat_clock_with_time_stops(
    started_at: Option<f64>,
    ended_at: &mut Option<f64>,
    time_stop: &TimeStopTracker,
) {
    let Some(started_at) = started_at else {
        return;
    };
    if let Some(timestamp) = time_stop.latest_game_pause_transition()
        && timestamp >= started_at
    {
        *ended_at = Some(ended_at.map_or(timestamp, |value| value.max(timestamp)));
    }
}

fn summarize_indexed_timeline(
    index: &CompactTimelineIndex,
    time_stop: &TimeStopTracker,
    start: Option<f64>,
    end: Option<f64>,
    markers: Vec<TimelineMarker>,
    options: TimelineAggregationOptions,
) -> TimelineSeries {
    let requested_bucket_seconds =
        if options.bucket_seconds.is_finite() && options.bucket_seconds > 0.0 {
            options.bucket_seconds
        } else {
            1.0
        };
    let Some(compact) = index.project_timeline(
        start,
        end,
        requested_bucket_seconds,
        options.max_buckets.max(1),
        options.max_roles_per_bucket.max(1),
        options.max_characters.max(1),
    ) else {
        return TimelineSeries {
            bucket_seconds: requested_bucket_seconds,
            markers,
            ..Default::default()
        };
    };
    let mut cumulative_damage = 0.0;
    // Timeline buckets use the configured wall-clock width even for the final
    // chart bucket; this preserves the existing contract's DPS denominator.
    let bucket_duration = compact.bucket_seconds.max(0.001);
    let buckets = compact
        .buckets
        .into_iter()
        .map(|bucket| {
            cumulative_damage += bucket.damage;
            TimelineBucket {
                start_offset: bucket.start_offset,
                end_offset: bucket.end_offset,
                damage: bucket.damage,
                dps: bucket.damage / bucket_duration,
                cumulative_damage,
                hits: bucket.hits,
                role_damage: bucket
                    .roles
                    .into_iter()
                    .map(|role| TimelineRoleBucket {
                        char_id: role.char_id,
                        char_name: role.char_name,
                        damage: role.damage,
                        dps: role.damage / bucket_duration,
                    })
                    .collect(),
            }
        })
        .collect();
    let (start, end) = compact
        .start_timestamp
        .zip(compact.end_timestamp)
        .unwrap_or((0.0, 0.0));
    let _subtract_time_stop = options.subtract_time_stop;
    TimelineSeries {
        bucket_seconds: compact.bucket_seconds,
        start_timestamp: compact.start_timestamp,
        end_timestamp: compact.end_timestamp,
        total_damage: cumulative_damage,
        omitted_role_damage: compact.omitted_role_damage,
        omitted_role_hits: compact.omitted_role_hits,
        buckets,
        time_stop_intervals: relative_time_stop_intervals(time_stop, start, end),
        compacted_time_stop_intervals: time_stop.compacted_interval_count(),
        markers,
    }
}

fn summarize_timeline_with_time_stop<'a, I>(
    hits: I,
    time_stop: &TimeStopTracker,
    start: Option<f64>,
    end: Option<f64>,
    markers: Vec<TimelineMarker>,
    options: TimelineAggregationOptions,
) -> TimelineSeries
where
    I: IntoIterator<Item = &'a Hit>,
{
    let requested_bucket_seconds =
        if options.bucket_seconds.is_finite() && options.bucket_seconds > 0.0 {
            options.bucket_seconds
        } else {
            1.0
        };
    let _subtract_time_stop = options.subtract_time_stop;
    let max_buckets = options.max_buckets.max(1);
    let max_roles_per_bucket = options.max_roles_per_bucket.max(1);
    let max_characters = options.max_characters.max(1);
    let (Some(start), Some(end)) = (start, end) else {
        return TimelineSeries {
            bucket_seconds: requested_bucket_seconds,
            markers,
            ..Default::default()
        };
    };
    // Subtracting two finite timestamps can still overflow to +infinity (for
    // example -f64::MAX..f64::MAX). Keep every subsequent count/allocation
    // calculation saturating and use a finite representable chart span.
    let raw_span = end - start;
    let span = if raw_span.is_finite() {
        raw_span.max(0.0)
    } else {
        f64::MAX
    };
    let requested_bucket_ratio = span / requested_bucket_seconds;
    let requested_bucket_count =
        if !requested_bucket_ratio.is_finite() || requested_bucket_ratio >= usize::MAX as f64 {
            usize::MAX
        } else {
            (requested_bucket_ratio.floor() as usize).saturating_add(1)
        };
    let (bucket_seconds, bucket_count) = if requested_bucket_count <= max_buckets {
        (requested_bucket_seconds, requested_bucket_count)
    } else {
        // The right edge belongs to the last bucket. Dividing the complete span
        // by the output budget ensures allocation is bounded before any bucket
        // or per-role HashMap is created, while retaining every hit in the
        // aggregate projection.
        (
            (span / max_buckets as f64).max(requested_bucket_seconds),
            max_buckets,
        )
    };
    let mut buckets = (0..bucket_count)
        .map(|index| TimelineBucket {
            start_offset: finite_timeline_offset(index, bucket_seconds),
            end_offset: finite_timeline_offset(index.saturating_add(1), bucket_seconds),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let mut role_buckets = vec![HashMap::<u32, (String, f64)>::new(); bucket_count];
    let mut retained_characters = HashSet::<u32>::with_capacity(max_characters);
    let mut omitted_role_damage = 0.0;
    let mut omitted_role_hits = 0_u64;

    for hit in hits {
        if hit.direction.is_incoming() || !hit.timestamp.is_finite() {
            continue;
        }
        let damage = hit.total_damage();
        if !damage.is_finite() {
            continue;
        }
        let raw_offset = hit.timestamp - start;
        let bucket_index = if raw_offset.is_finite() {
            ((raw_offset.max(0.0) / bucket_seconds).floor() as usize).min(bucket_count - 1)
        } else {
            // Halving before subtraction keeps the full finite f64 domain
            // representable, then maps the relative position into the already
            // bounded bucket set without allocating an intermediate axis.
            let scaled_span = end / 2.0 - start / 2.0;
            let relative = if scaled_span.is_finite() && scaled_span > 0.0 {
                ((hit.timestamp / 2.0 - start / 2.0) / scaled_span).clamp(0.0, 1.0)
            } else {
                0.0
            };
            ((relative * bucket_count as f64).floor() as usize).min(bucket_count - 1)
        };
        let bucket = &mut buckets[bucket_index];
        bucket.damage += damage;
        bucket.hits += 1;
        let roles = &mut role_buckets[bucket_index];
        if let Some(role) = roles.get_mut(&hit.char_id) {
            role.0.clone_from(&hit.char_name);
            role.1 += damage;
        } else if roles.len() < max_roles_per_bucket
            && (retained_characters.contains(&hit.char_id)
                || retained_characters.len() < max_characters)
        {
            retained_characters.insert(hit.char_id);
            roles.insert(hit.char_id, (hit.char_name.clone(), damage));
        } else {
            omitted_role_damage += damage;
            omitted_role_hits = omitted_role_hits.saturating_add(1);
        }
    }

    let mut total_damage = 0.0;
    for (index, bucket) in buckets.iter_mut().enumerate() {
        total_damage += bucket.damage;
        bucket.cumulative_damage = total_damage;
        // Timeline buckets stay on real wall-clock seconds. Time-stop periods
        // are drawn as bands; subtracting them inside a fixed 1s bucket can
        // shrink the divisor to almost zero and produce unusable peak spikes.
        let duration = bucket_seconds.max(0.001);
        bucket.dps = bucket.damage / duration;
        let mut roles = role_buckets[index]
            .drain()
            .map(|(char_id, (char_name, damage))| TimelineRoleBucket {
                char_id,
                char_name,
                damage,
                dps: damage / duration,
            })
            .collect::<Vec<_>>();
        roles.sort_by(|left, right| {
            right
                .damage
                .total_cmp(&left.damage)
                .then_with(|| left.char_name.cmp(&right.char_name))
                .then_with(|| left.char_id.cmp(&right.char_id))
        });
        bucket.role_damage = roles;
    }

    TimelineSeries {
        bucket_seconds,
        start_timestamp: Some(start),
        end_timestamp: Some(end),
        total_damage,
        omitted_role_damage,
        omitted_role_hits,
        buckets,
        time_stop_intervals: relative_time_stop_intervals(time_stop, start, end),
        compacted_time_stop_intervals: time_stop.compacted_interval_count(),
        markers,
    }
}

fn finite_timeline_offset(index: usize, bucket_seconds: f64) -> f64 {
    let offset = index as f64 * bucket_seconds;
    if offset.is_finite() { offset } else { f64::MAX }
}

fn relative_time_stop_intervals(
    time_stop: &TimeStopTracker,
    start: f64,
    end: f64,
) -> Vec<TimelineTimeStopInterval> {
    time_stop
        .intervals_between(start, end)
        .into_iter()
        .map(|interval| TimelineTimeStopInterval {
            start_offset: interval.start - start,
            end_offset: interval.end - start,
        })
        .collect()
}

fn push_timeline_marker(
    markers: &mut Vec<TimelineMarker>,
    timestamp: Option<f64>,
    start: f64,
    end: f64,
    label: &str,
    kind: TimelineMarkerKind,
) {
    if markers.len() >= MAX_TIMELINE_MARKERS {
        return;
    }
    let Some(timestamp) = timestamp else {
        return;
    };
    if !timestamp.is_finite() {
        return;
    }
    let timestamp = timestamp.clamp(start, end);
    markers.push(TimelineMarker {
        offset: timestamp - start,
        label: label.to_owned(),
        kind,
    });
}

fn sort_timeline_markers(markers: &mut [TimelineMarker]) {
    markers.sort_by(|left, right| {
        left.offset
            .total_cmp(&right.offset)
            .then_with(|| left.label.cmp(&right.label))
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModScriptEventPhase {
    Event,
    Preprocess,
    Postprocess,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnemyIdentity {
    pub config_hash: u64,
    pub config_id: String,
    pub monster_id: String,
    pub name_en: String,
    pub name_zh: String,
    pub name_ja: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModScriptEvent {
    pub sequence: u64,
    pub timestamp_100ns: u64,
    pub mod_id: String,
    pub phase: ModScriptEventPhase,
    pub name: String,
    pub values: Vec<u64>,
    pub enemy_identity: Option<EnemyIdentity>,
}

impl ModScriptEvent {
    pub fn from_bridge(
        sequence: u64,
        timestamp_100ns: u64,
        mod_id: String,
        name: String,
        values: Vec<u64>,
    ) -> Self {
        let (phase, name) = if let Some(name) = name.strip_prefix("pre.") {
            (ModScriptEventPhase::Preprocess, name.to_owned())
        } else if let Some(name) = name.strip_prefix("post.") {
            (ModScriptEventPhase::Postprocess, name.to_owned())
        } else {
            (ModScriptEventPhase::Event, name)
        };
        Self {
            sequence,
            timestamp_100ns,
            mod_id,
            phase,
            name,
            values,
            enemy_identity: None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum EngineEvent {
    Hit(Box<Hit>),
    HitFollowUp(HitFollowUp),
    HitDamageCorrection(HitDamageCorrection),
    UnattributedServerDamage(UnattributedServerDamage),
    Packet(Box<PacketDebug>),
    PacketObservation(PacketObservation),
    Abyss(AbyssEvent),
    TimeStop(TimeStopEvent),
    CombatClockHealth(CombatClockRuntimeHealth),
    EmptyCurtain(Vec<EmptyCurtainItem>),
    EmptyCurtainCharacters(Vec<EmptyCurtainCharacter>),
    ModScript(ModScriptEvent),
    Status(String),
    Warning(String),
    Error(String),
    CaptureStopped,
}

impl EngineEvent {
    pub fn is_droppable_debug_packet(&self) -> bool {
        matches!(self, Self::Packet(_))
    }
}

#[derive(Clone, Copy, Debug)]
struct HitAggregateContribution {
    char_id: u32,
    timestamp: f64,
    total_damage: f64,
    max_hp_reduction: f64,
    incoming: bool,
    character_counted: bool,
    attributed: bool,
    direct_counted: bool,
    direct_damage: f64,
    hud_visible: bool,
}

impl From<&Hit> for HitAggregateContribution {
    fn from(hit: &Hit) -> Self {
        let incoming = hit.direction.is_incoming();
        let character_counted = !incoming && !is_unbalance_damage_hit(hit);
        let attributed =
            character_counted && matches!(hit.direction, HitDirection::Outgoing) && hit.char_known;
        let direct_damage = if attributed {
            direct_damage_for_hit(hit)
        } else {
            0.0
        };
        Self {
            char_id: hit.char_id,
            timestamp: hit.timestamp,
            total_damage: hit.total_damage(),
            max_hp_reduction: if incoming {
                0.0
            } else {
                hit.max_hp_reduction.max(0.0)
            },
            incoming,
            character_counted,
            attributed,
            direct_counted: direct_damage > 0.0,
            direct_damage,
            hud_visible: hit.char_known || !is_qte_follow_up_damage_hit(hit),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct HitAggregateMutation {
    before: HitAggregateContribution,
    after: HitAggregateContribution,
    after_source: HitSourceIdentity,
}

impl HitAggregateMutation {
    fn new(before: HitAggregateContribution, hit: &Hit) -> Self {
        Self {
            before,
            after: HitAggregateContribution::from(hit),
            after_source: HitSourceIdentity::from(hit),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HitLocator {
    char_id: u32,
    timestamp_bits: u64,
    byte_offset: usize,
    bit_shift: u8,
    gameplay_effect_index: Option<u32>,
    target_max_hp_bits: u64,
}

impl HitLocator {
    fn matches(self, hit: &Hit) -> bool {
        hit.char_id == self.char_id
            && hit.timestamp.to_bits() == self.timestamp_bits
            && hit.byte_offset == self.byte_offset
            && hit.bit_shift == self.bit_shift
            && hit.gameplay_effect_index == self.gameplay_effect_index
            && hit.target_max_hp.to_bits() == self.target_max_hp_bits
    }
}

impl From<&Hit> for HitLocator {
    fn from(hit: &Hit) -> Self {
        Self {
            char_id: hit.char_id,
            timestamp_bits: hit.timestamp.to_bits(),
            byte_offset: hit.byte_offset,
            bit_shift: hit.bit_shift,
            gameplay_effect_index: hit.gameplay_effect_index,
            target_max_hp_bits: hit.target_max_hp.to_bits(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct RecentHitRecord {
    locator: HitLocator,
    sources: [Option<HitSourceIdentity>; RECENT_HIT_SOURCE_ALIASES],
    next_source: usize,
    abyss_half: Option<AbyssHalf>,
}

impl RecentHitRecord {
    fn from_hit(hit: &Hit, abyss_half: Option<AbyssHalf>) -> Self {
        Self::from_source(
            HitLocator::from(hit),
            HitSourceIdentity::from(hit),
            abyss_half,
        )
    }

    fn from_source(
        locator: HitLocator,
        source: HitSourceIdentity,
        abyss_half: Option<AbyssHalf>,
    ) -> Self {
        let mut sources = [None; RECENT_HIT_SOURCE_ALIASES];
        sources[0] = Some(source);
        Self {
            locator,
            sources,
            next_source: 1,
            abyss_half,
        }
    }

    fn matches_source(self, source: HitSourceIdentity) -> bool {
        self.sources
            .iter()
            .flatten()
            .any(|candidate| candidate.matches_source(source))
    }

    fn remember_source(&mut self, source: HitSourceIdentity) {
        if self.matches_source(source) {
            return;
        }
        if let Some(index) = self.sources.iter().position(Option::is_none) {
            self.sources[index] = Some(source);
            self.next_source = (index + 1) % RECENT_HIT_SOURCE_ALIASES;
            return;
        }
        self.sources[self.next_source] = Some(source);
        self.next_source = (self.next_source + 1) % RECENT_HIT_SOURCE_ALIASES;
    }
}

fn push_recent_hit_record(records: &mut VecDeque<RecentHitRecord>, record: RecentHitRecord) {
    records.push_back(record);
    while records.len() > RECENT_HIT_MUTATION_WINDOW {
        records.pop_front();
    }
}

fn remember_recent_hit(
    records: &mut VecDeque<RecentHitRecord>,
    hit: &Hit,
    abyss_half: Option<AbyssHalf>,
) {
    push_recent_hit_record(records, RecentHitRecord::from_hit(hit, abyss_half));
}

fn find_recent_hit(hits: &VecDeque<Hit>, locator: HitLocator) -> Option<&Hit> {
    hits.iter()
        .rev()
        .take(RECENT_HIT_MUTATION_WINDOW)
        .find(|hit| locator.matches(hit))
}

fn find_recent_hit_position(hits: &VecDeque<Hit>, locator: HitLocator) -> Option<usize> {
    hits.iter()
        .rev()
        .take(RECENT_HIT_MUTATION_WINDOW)
        .position(|hit| locator.matches(hit))
        .map(|reverse_index| hits.len().saturating_sub(reverse_index + 1))
}

fn find_recent_hit_mut(hits: &mut VecDeque<Hit>, locator: HitLocator) -> Option<&mut Hit> {
    hits.iter_mut()
        .rev()
        .take(RECENT_HIT_MUTATION_WINDOW)
        .find(|hit| locator.matches(hit))
}

fn recent_hits_contain_locator(hits: &VecDeque<Hit>, locator: HitLocator) -> bool {
    find_recent_hit(hits, locator).is_some()
}

fn find_recent_hit_locator(hits: &VecDeque<Hit>, source: HitSourceIdentity) -> Option<HitLocator> {
    hits.iter()
        .rev()
        .take(RECENT_HIT_MUTATION_WINDOW)
        .find(|hit| source.matches_hit(hit))
        .map(HitLocator::from)
}

fn apply_follow_up_to_recent_hit(
    hits: &mut VecDeque<Hit>,
    locator: HitLocator,
    follow_up: &HitFollowUp,
) -> Option<HitAggregateMutation> {
    let hit = find_recent_hit_mut(hits, locator)?;
    let next_follow_up_damage = hit.follow_up_damage + follow_up.damage;
    let changed = hit.follow_up_damage.to_bits() != next_follow_up_damage.to_bits()
        || hit.follow_up_timestamp.map(f64::to_bits) != Some(follow_up.timestamp.to_bits())
        || hit.follow_up_damage_name != follow_up.damage_name
        || hit.follow_up_attack_type != follow_up.attack_type
        || hit.follow_up_damage_attribute != follow_up.damage_attribute
        || hit.target_hp_after.to_bits() != follow_up.target_hp_after.to_bits()
        || hit.target_hp_percent.to_bits() != follow_up.target_hp_percent.to_bits();
    if !changed {
        return None;
    }
    let before = HitAggregateContribution::from(&*hit);
    hit.follow_up_damage = next_follow_up_damage;
    hit.follow_up_timestamp = Some(follow_up.timestamp);
    hit.follow_up_damage_name.clone_from(&follow_up.damage_name);
    hit.follow_up_attack_type.clone_from(&follow_up.attack_type);
    hit.follow_up_damage_attribute
        .clone_from(&follow_up.damage_attribute);
    hit.target_hp_after = follow_up.target_hp_after;
    hit.target_hp_percent = follow_up.target_hp_percent;
    Some(HitAggregateMutation::new(before, hit))
}

fn apply_damage_correction_to_recent_hit(
    hits: &mut VecDeque<Hit>,
    locator: HitLocator,
    correction: &HitDamageCorrection,
) -> Option<HitAggregateMutation> {
    let hit = find_recent_hit_mut(hits, locator)?;
    let overkill_changed = correction
        .reconciled_overkill_damage
        .is_some_and(|overkill| {
            hit.reconciled_overkill_damage.map(f64::to_bits) != Some(overkill.to_bits())
        });
    let max_hp_reduction_changed = correction
        .max_hp_reduction
        .is_some_and(|reduction| hit.max_hp_reduction.to_bits() != reduction.to_bits());
    let wire_overkill_interval_retired =
        correction.reconciled_overkill_damage.is_some() && hit.wire_event.is_some();
    let changed = hit.damage.to_bits() != correction.damage.to_bits()
        || hit.target_hp_before.to_bits() != correction.target_hp_before.to_bits()
        || hit.target_hp_after.to_bits() != correction.target_hp_after.to_bits()
        || hit.target_hp_percent.to_bits() != correction.target_hp_percent.to_bits()
        || overkill_changed
        || max_hp_reduction_changed
        || wire_overkill_interval_retired;
    if !changed {
        return None;
    }
    let before = HitAggregateContribution::from(&*hit);
    hit.damage = correction.damage;
    hit.target_hp_before = correction.target_hp_before;
    hit.target_hp_after = correction.target_hp_after;
    hit.target_hp_percent = correction.target_hp_percent;
    if let Some(reduction) = correction.max_hp_reduction {
        hit.max_hp_reduction = reduction;
    }
    if correction.reconciled_overkill_damage.is_some() {
        hit.reconciled_overkill_damage = correction.reconciled_overkill_damage;
        // The server has finalized this hit's effective/overkill split. Keeping
        // the original client wire interval would let a later peer recompute
        // and overwrite that authoritative result from its stale HP snapshot.
        hit.wire_event = None;
    }
    Some(HitAggregateMutation::new(before, hit))
}

/// Identifies the `Hit` a follow-up or damage correction was derived from.
///
/// Requires every field to still match, including `gameplay_effect_index`
/// when both sides have one: that index is a per-application identifier, not
/// a per-hit one, so an AoE or multi-tick effect can hand out the same index
/// to several hits with different targets/HP in the same packet. Matching on
/// the index alone (without the HP/damage identity) risked picking whichever
/// same-index hit happened to be found first instead of the right one — the
/// damage/HP reconciliation mechanisms are now mutually exclusive per boss-HP
/// update (see `PacketDecoder::reconcile_boss_hp_updates`) specifically so a
/// hit's fields never get mutated out from under a still-pending match.
#[derive(Clone, Copy, Debug)]
struct HitSourceIdentity {
    char_id: u32,
    timestamp: f64,
    gameplay_effect_index: Option<u32>,
    damage: f64,
    target_hp_before: f64,
    target_hp_after: f64,
    target_max_hp: f64,
}

impl HitSourceIdentity {
    fn matches_hit(self, hit: &Hit) -> bool {
        hit.char_id == self.char_id
            && (hit.timestamp - self.timestamp).abs() <= 0.001
            && hit.gameplay_effect_index == self.gameplay_effect_index
            && (hit.damage - self.damage).abs() <= 0.5
            && (hit.target_hp_before - self.target_hp_before).abs() <= 0.5
            && (hit.target_hp_after - self.target_hp_after).abs() <= 0.5
            && (hit.target_max_hp - self.target_max_hp).abs() <= 0.5
    }

    fn matches_source(self, other: Self) -> bool {
        self.char_id == other.char_id
            && (self.timestamp - other.timestamp).abs() <= 0.001
            && self.gameplay_effect_index == other.gameplay_effect_index
            && (self.damage - other.damage).abs() <= 0.5
            && (self.target_hp_before - other.target_hp_before).abs() <= 0.5
            && (self.target_hp_after - other.target_hp_after).abs() <= 0.5
            && (self.target_max_hp - other.target_max_hp).abs() <= 0.5
    }
}

impl From<&Hit> for HitSourceIdentity {
    fn from(hit: &Hit) -> Self {
        Self {
            char_id: hit.char_id,
            timestamp: hit.timestamp,
            gameplay_effect_index: hit.gameplay_effect_index,
            damage: hit.damage,
            target_hp_before: hit.target_hp_before,
            target_hp_after: hit.target_hp_after,
            target_max_hp: hit.target_max_hp,
        }
    }
}

impl From<&HitFollowUp> for HitSourceIdentity {
    fn from(follow_up: &HitFollowUp) -> Self {
        Self {
            char_id: follow_up.source_char_id,
            timestamp: follow_up.source_timestamp,
            gameplay_effect_index: follow_up.source_gameplay_effect_index,
            damage: follow_up.source_damage,
            target_hp_before: follow_up.source_target_hp_before,
            target_hp_after: follow_up.source_target_hp_after,
            target_max_hp: follow_up.source_target_max_hp,
        }
    }
}

impl From<&HitDamageCorrection> for HitSourceIdentity {
    fn from(correction: &HitDamageCorrection) -> Self {
        Self {
            char_id: correction.source_char_id,
            timestamp: correction.source_timestamp,
            gameplay_effect_index: correction.source_gameplay_effect_index,
            damage: correction.source_damage,
            target_hp_before: correction.source_target_hp_before,
            target_hp_after: correction.source_target_hp_after,
            target_max_hp: correction.source_target_max_hp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_script_bridge_classifies_preprocess_and_postprocess_names() {
        for (wire_name, phase, name) in [
            ("pre.hit", ModScriptEventPhase::Preprocess, "hit"),
            ("post.summary", ModScriptEventPhase::Postprocess, "summary"),
            (
                "character.health",
                ModScriptEventPhase::Event,
                "character.health",
            ),
        ] {
            let event = ModScriptEvent::from_bridge(
                7,
                11,
                "example".to_owned(),
                wire_name.to_owned(),
                vec![1, 2],
            );

            assert_eq!(event.phase, phase);
            assert_eq!(event.name, name);
            assert_eq!(event.values, vec![1, 2]);
        }
    }

    #[test]
    fn team_dps_export_is_compact_and_roundtrips() {
        let export = TeamDpsExport {
            version: TEAM_DPS_EXPORT_VERSION,
            single: None,
            upper: Some(TeamDps {
                dps: 31535.0,
                members: vec![TeamDpsMember {
                    id: 1010,
                    dps: 23700.0,
                    name: "娜娜莉".to_owned(),
                }],
            }),
            lower: None,
        };
        let json = serde_json::to_string(&export).unwrap();
        // Compact (no pretty newlines) and absent teams are omitted to stay small.
        assert!(!json.contains('\n'));
        assert!(!json.contains("single"));
        assert!(!json.contains("lower"));

        let parsed: TeamDpsExport = serde_json::from_str(&json).unwrap();
        assert!(parsed.single.is_none());
        assert_eq!(parsed.upper.as_ref().unwrap().members[0].id, 1010);
        assert_eq!(parsed.upper.as_ref().unwrap().members.len(), 1);
    }

    #[test]
    fn team_dps_export_version_defaults_when_missing() {
        let parsed: TeamDpsExport = serde_json::from_str(r#"{"single":{"dps":100.0}}"#).unwrap();
        assert_eq!(parsed.version, TEAM_DPS_EXPORT_VERSION);
        assert_eq!(parsed.single.unwrap().dps, 100.0);
    }

    #[test]
    fn hit_direction_json_values_roundtrip() {
        for (direction, json) in [
            (HitDirection::Outgoing, r#""outgoing""#),
            (HitDirection::Incoming, r#""incoming""#),
            (HitDirection::Unknown, r#""unknown""#),
        ] {
            assert_eq!(serde_json::to_string(&direction).unwrap(), json);
            assert_eq!(
                serde_json::from_str::<HitDirection>(json).unwrap(),
                direction
            );
        }
    }

    #[test]
    fn hit_direction_json_rejects_invalid_value() {
        assert!(serde_json::from_str::<HitDirection>(r#""sideways""#).is_err());
    }

    #[test]
    fn hit_character_source_json_values_roundtrip() {
        for (source, json) in [
            (HitCharacterSource::Packet, r#""packet""#),
            (HitCharacterSource::Session, r#""session""#),
            (HitCharacterSource::GameplayEffect, r#""gameplay_effect""#),
            (HitCharacterSource::ExportJson, r#""export_json""#),
            (HitCharacterSource::Unknown, r#""unknown""#),
        ] {
            assert_eq!(serde_json::to_string(&source).unwrap(), json);
            assert_eq!(
                serde_json::from_str::<HitCharacterSource>(json).unwrap(),
                source
            );
        }
    }

    #[test]
    fn hit_character_source_json_rejects_invalid_value() {
        assert!(serde_json::from_str::<HitCharacterSource>(r#""heuristic""#).is_err());
    }

    #[test]
    fn abyss_half_json_is_stable_and_accepts_legacy_labels() {
        assert_eq!(
            serde_json::to_string(&AbyssHalf::First).unwrap(),
            r#""first""#
        );
        assert_eq!(
            serde_json::to_string(&AbyssHalf::Second).unwrap(),
            r#""second""#
        );
        for value in [r#""Ascending Line""#, r#""上行线""#, r#""上りライン""#] {
            assert_eq!(
                serde_json::from_str::<AbyssHalf>(value).unwrap(),
                AbyssHalf::First
            );
        }
        for value in [r#""Descending Line""#, r#""下行线""#, r#""下りライン""#] {
            assert_eq!(
                serde_json::from_str::<AbyssHalf>(value).unwrap(),
                AbyssHalf::Second
            );
        }
    }

    #[test]
    fn abyss_half_json_rejects_invalid_value() {
        assert!(serde_json::from_str::<AbyssHalf>(r#""middle""#).is_err());
    }

    #[test]
    fn dps_time_basis_json_is_stable_and_accepts_legacy_labels() {
        assert_eq!(
            serde_json::to_string(&DpsTimeBasis::SubtractTimeStop).unwrap(),
            r#""subtract_time_stop""#
        );
        assert_eq!(
            serde_json::to_string(&DpsTimeBasis::WallClock).unwrap(),
            r#""wall_clock""#
        );
        for value in [
            r#""time_stop_adjusted""#,
            r#""Exclude Time Stop""#,
            r#""扣除时停""#,
            r#""時間停止を除外""#,
        ] {
            assert_eq!(
                serde_json::from_str::<DpsTimeBasis>(value).unwrap(),
                DpsTimeBasis::SubtractTimeStop
            );
        }
        for value in [
            r#""real_time""#,
            r#""Real Time""#,
            r#""实时""#,
            r#""现实时间""#,
            r#""実時間""#,
        ] {
            assert_eq!(
                serde_json::from_str::<DpsTimeBasis>(value).unwrap(),
                DpsTimeBasis::WallClock
            );
        }
        assert!(DpsTimeBasis::from_subtract_time_stop(true).subtracts_time_stop());
        assert!(!DpsTimeBasis::from_subtract_time_stop(false).subtracts_time_stop());
    }

    #[test]
    fn dps_time_basis_json_rejects_invalid_value() {
        assert!(serde_json::from_str::<DpsTimeBasis>(r#""paused_only""#).is_err());
    }

    fn test_hit(timestamp: f64, char_id: u32, direction: &str, damage: f64) -> Hit {
        Hit {
            timestamp,
            char_id,
            char_name: format!("角色{char_id}"),
            char_known: true,
            damage,
            byte_offset: 0,
            bit_shift: 0,
            char_source: HitCharacterSource::Unknown,
            direction: HitDirection::try_from(direction).expect("test direction must be valid"),
            target_hp_before: 0.0,
            target_hp_after: 0.0,
            target_max_hp: 0.0,
            max_hp_reduction: 0.0,
            target_hp_percent: 0.0,
            target_id: None,
            target_name: None,
            target_name_en: None,
            target_name_ja: None,
            target_monster_id: None,
            target_context: Vec::new(),
            gameplay_effect_index: None,
            gameplay_effect_name: None,
            ability_name: None,
            damage_name: None,
            damage_component: None,
            attack_type: None,
            damage_attribute: None,
            follow_up_damage: 0.0,
            follow_up_timestamp: None,
            follow_up_damage_name: None,
            follow_up_attack_type: None,
            follow_up_damage_attribute: None,
            reconciled_overkill_damage: None,
            wire_event: None,
        }
    }

    #[test]
    fn overkill_damage_records_only_primary_damage_beyond_known_target_hp() {
        let mut hit = test_hit(1.0, 1010, "outgoing", 1_500.0);
        hit.target_hp_before = 1_000.0;
        hit.target_max_hp = 10_000.0;
        hit.follow_up_damage = 250.0;
        assert_eq!(hit.overkill_damage(), 500.0);

        hit.damage = 1_000.0;
        assert_eq!(hit.overkill_damage(), 0.0);

        hit.damage = 1_500.0;
        hit.target_max_hp = 0.0;
        assert_eq!(hit.overkill_damage(), 0.0);
    }

    #[test]
    fn overkill_damage_is_zero_while_authoritative_target_hp_remains_positive() {
        let mut hit = test_hit(1.0, 1004, "outgoing", 224_701.0);
        hit.target_hp_before = 711_968.0;
        hit.target_hp_after = 391_749.0;
        hit.target_max_hp = 2_292_536.0;
        hit.reconciled_overkill_damage = Some(169_224.0);

        assert_eq!(hit.overkill_damage(), 0.0);
    }

    #[test]
    fn overkill_reconciles_hits_that_share_one_target_hp_interval() {
        fn interval_hit(timestamp: f64, damage_time: f64, damage: f64) -> Hit {
            let mut hit = test_hit(timestamp, 1010, "outgoing", damage);
            hit.target_id = Some("enemy-wire:test".to_owned());
            hit.target_hp_before = 72_727.0;
            hit.target_hp_after = (72_727.0 - damage).max(0.0);
            hit.target_max_hp = 398_200.0;
            hit.target_hp_percent = hit.target_hp_after / hit.target_max_hp * 100.0;
            hit.wire_event = Some(DamageWireEvent {
                damage: damage as f32,
                target_hp_before: 72_727.0,
                target_max_hp: 398_200.0,
                damage_time,
                world_time: 100.0,
                repeated_damage: damage as f32,
                state_flags: [0, 1, 0],
                trailing_value: 0.0,
            });
            hit
        }

        let mut state = CombatState::default();
        // Capture delivery can be opposite to the exact wire order. The
        // interval is re-sorted by damage_time whenever a late peer arrives.
        state.push_hit(interval_hit(2.0, 20.002, 76_702.0));
        state.push_hit(interval_hit(2.001, 20.001, 4_926.0));

        let small = state
            .hits
            .iter()
            .find(|hit| hit.damage == 4_926.0)
            .expect("small hit should remain in the interval");
        let lethal = state
            .hits
            .iter()
            .find(|hit| hit.damage == 76_702.0)
            .expect("lethal hit should remain in the interval");
        assert_eq!(small.overkill_damage(), 0.0);
        assert_eq!(lethal.overkill_damage(), 8_901.0);
        assert_eq!(
            state.hits.iter().map(Hit::overkill_damage).sum::<f64>(),
            8_901.0
        );
    }

    #[test]
    fn server_overkill_correction_survives_later_hit_in_same_wire_interval() {
        fn interval_hit(timestamp: f64, damage_time: f64, damage: f64) -> Hit {
            let mut hit = test_hit(timestamp, 1010, "outgoing", damage);
            hit.target_id = Some("enemy-wire:server-correction".to_owned());
            hit.target_hp_before = 55_477.0;
            hit.target_hp_after = (55_477.0 - damage).max(0.0);
            hit.target_max_hp = 2_292_536.0;
            hit.target_hp_percent = hit.target_hp_after / hit.target_max_hp * 100.0;
            hit.wire_event = Some(DamageWireEvent {
                damage: damage as f32,
                target_hp_before: 55_477.0,
                target_max_hp: 2_292_536.0,
                damage_time,
                world_time: 100.0,
                repeated_damage: damage as f32,
                state_flags: [0, 1, 0],
                trailing_value: 0.0,
            });
            hit
        }

        let mut state = CombatState::default();
        state.push_hit(interval_hit(10.0, 20.0, 55_477.0));
        assert!(state.apply_damage_correction(HitDamageCorrection {
            source_timestamp: 10.0,
            source_char_id: 1010,
            source_damage: 55_477.0,
            source_target_hp_before: 55_477.0,
            source_target_hp_after: 0.0,
            source_target_max_hp: 2_292_536.0,
            source_gameplay_effect_index: None,
            damage: 224_701.0,
            target_hp_before: 616_450.0,
            target_hp_after: 391_749.0,
            target_hp_percent: 391_749.0 / 2_292_536.0 * 100.0,
            max_hp_reduction: Some(449_402.0),
            reconciled_overkill_damage: Some(0.0),
        }));

        state.push_hit(interval_hit(10.01, 20.01, 7_069.0));

        assert_eq!(state.hits[0].overkill_damage(), 0.0);
        assert_eq!(state.hits[1].overkill_damage(), 0.0);
        assert_eq!(state.hits[0].target_hp_after, 391_749.0);
    }

    fn apply_test_pause(state: &mut CombatState, start: f64, end: f64) {
        state.apply_time_stop_event(TimeStopEvent::GamePauseStarted {
            timestamp: start,
            pause_type_mask: 1 << 2,
        });
        state.apply_time_stop_event(TimeStopEvent::GamePauseEnded {
            timestamp: end,
            pause_type_mask: 1 << 2,
        });
    }

    #[test]
    fn history_archive_snapshot_preserves_summary_without_runtime_payloads() {
        let mut state = CombatState::default();
        state.push_hit(test_hit(1.0, 7, "outgoing", 123.0));
        state.push_hit(test_hit(2.0, 8, "incoming", 45.0));
        state.push_packet(debug_packet(1, 256));
        state.observe_packet(PacketObservation { parsed_hits: 1 });
        state.empty_curtain.push(EmptyCurtainItem {
            id: HtItemNetId::ZERO,
            item_id: "inventory-only".to_owned(),
            level: 1,
            main_stats: Vec::new(),
            sub_stats: Vec::new(),
            locked: false,
            discarded: false,
            character_net_id: None,
            equipped_character_id: None,
            equipped_placement: None,
        });
        state.empty_curtain_characters.push(EmptyCurtainCharacter {
            net_id: HtItemNetId::ZERO,
            character_id: 7,
        });
        apply_test_pause(&mut state, 1.25, 1.75);
        let expected = state
            .session_summary(
                CaptureQualitySource::Live,
                DpsTimeBasis::SubtractTimeStop,
                false,
            )
            .expect("source History summary");
        assert!(!state.compact_timeline.team.nodes.is_empty());

        let snapshot = state.clone_for_history_archive();

        assert_eq!(snapshot.hits.len(), state.hits.len());
        assert!(snapshot.packets.is_empty());
        assert_eq!(snapshot.packet_count, state.packet_count);
        assert_eq!(snapshot.packets_with_hits, state.packets_with_hits);
        assert!(snapshot.empty_curtain.is_empty());
        assert!(snapshot.empty_curtain_characters.is_empty());
        assert!(snapshot.compact_timeline.team.nodes.is_empty());
        assert!(snapshot.compact_timeline.roles.is_empty());
        assert!(snapshot.recent_hit_records.is_empty());
        assert_eq!(
            snapshot.session_summary(
                CaptureQualitySource::Live,
                DpsTimeBasis::SubtractTimeStop,
                false,
            ),
            Some(expected)
        );
    }

    fn reset_combat_total_rebuild_count() {
        COMBAT_TOTAL_REBUILD_COUNT.with(|count| count.set(0));
    }

    fn combat_total_rebuild_count() -> usize {
        COMBAT_TOTAL_REBUILD_COUNT.with(std::cell::Cell::get)
    }

    fn reset_combat_detail_query_visits() {
        COMBAT_DETAIL_QUERY_VISITS.with(|visits| visits.set(0));
    }

    fn combat_detail_query_visits() -> usize {
        COMBAT_DETAIL_QUERY_VISITS.with(std::cell::Cell::get)
    }

    fn reset_bulk_combat_rebuild_visits() {
        BULK_COMBAT_REBUILD_VISITS.with(|visits| visits.set(0));
    }

    fn bulk_combat_rebuild_visits() -> usize {
        BULK_COMBAT_REBUILD_VISITS.with(std::cell::Cell::get)
    }

    fn reset_compact_timeline_projection_visits() {
        COMPACT_TIMELINE_PROJECTION_VISITS.with(|visits| visits.set(0));
    }

    fn compact_timeline_projection_visits() -> usize {
        COMPACT_TIMELINE_PROJECTION_VISITS.with(std::cell::Cell::get)
    }

    fn reset_compact_timeline_role_scratch_slots() {
        COMPACT_TIMELINE_ROLE_SCRATCH_SLOTS.with(|slots| slots.set(0));
    }

    fn compact_timeline_role_scratch_slots() -> usize {
        COMPACT_TIMELINE_ROLE_SCRATCH_SLOTS.with(std::cell::Cell::get)
    }

    fn reset_compact_timeline_role_candidate_slots() {
        COMPACT_TIMELINE_ROLE_CANDIDATE_SLOTS.with(|slots| slots.set(0));
    }

    fn compact_timeline_role_candidate_slots() -> usize {
        COMPACT_TIMELINE_ROLE_CANDIDATE_SLOTS.with(std::cell::Cell::get)
    }

    fn scale_dense_timeline_tree(
        tree: &mut CompactTimelineTree,
        damage_factor: f64,
        hits_factor: u64,
        sequence_stride: u64,
        sequence_offset: u64,
    ) {
        for node in &mut tree.nodes {
            let aggregate = match node {
                CompactTimelineTreeNode::Leaf { aggregate, .. }
                | CompactTimelineTreeNode::Branch { aggregate, .. } => aggregate,
            };
            aggregate.damage *= damage_factor;
            aggregate.hits = aggregate.hits.saturating_mul(hits_factor);
            aggregate.first_sequence = aggregate
                .first_sequence
                .saturating_mul(sequence_stride)
                .saturating_add(sequence_offset);
        }
    }

    #[test]
    fn only_full_debug_packets_are_droppable_under_backpressure() {
        let packet = PacketDebug {
            timestamp: 1.0,
            source: "127.0.0.1:1".to_owned(),
            destination: "127.0.0.1:2".to_owned(),
            direction: "outgoing".to_owned(),
            payload_len: 0,
            declared_ids: Vec::new(),
            parsed_hits: 0,
            note: String::new(),
            payload_preview: String::new(),
            payload_hex: String::new(),
            decoded_text: String::new(),
        };

        assert!(EngineEvent::Packet(Box::new(packet)).is_droppable_debug_packet());
        assert!(
            !EngineEvent::PacketObservation(PacketObservation { parsed_hits: 0 })
                .is_droppable_debug_packet()
        );
        assert!(
            !EngineEvent::Hit(Box::new(test_hit(1.0, 1, "outgoing", 10.0)))
                .is_droppable_debug_packet()
        );
        assert!(!EngineEvent::EmptyCurtain(Vec::new()).is_droppable_debug_packet());
        assert!(!EngineEvent::CaptureStopped.is_droppable_debug_packet());
    }

    fn debug_packet(index: usize, payload_hex_bytes: usize) -> PacketDebug {
        PacketDebug {
            timestamp: index as f64,
            source: "127.0.0.1:1".to_owned(),
            destination: "127.0.0.1:2".to_owned(),
            direction: "outgoing".to_owned(),
            payload_len: payload_hex_bytes / 2,
            declared_ids: vec![index as u32],
            parsed_hits: 0,
            note: format!("packet-{index}"),
            payload_preview: String::new(),
            payload_hex: "A".repeat(payload_hex_bytes),
            decoded_text: String::new(),
        }
    }

    #[test]
    fn debug_packet_ring_is_bounded_by_items_and_retained_bytes() {
        let mut state = CombatState::default();
        for index in 0..20 {
            assert!(state.push_packet(debug_packet(index, 1024 * 1024)));
        }

        assert!(state.packets.len() < 20);
        assert!(state.packets.len() <= MAX_DEBUG_PACKETS);
        assert!(state.packet_debug_bytes <= MAX_DEBUG_PACKET_BYTES);
        assert_eq!(
            state.packet_debug_bytes,
            state
                .packets
                .iter()
                .map(PacketDebug::retained_heap_bytes)
                .sum::<usize>()
        );
        assert_eq!(
            state.packets.back().map(|packet| packet.note.as_str()),
            Some("packet-19")
        );

        let generation = state.packets_generation;
        let mut oversized = debug_packet(99, 0);
        oversized.payload_hex = String::with_capacity(MAX_DEBUG_PACKET_BYTES + 1);
        assert!(!state.push_packet(oversized));
        assert_eq!(state.packets_generation, generation);
        assert!(state.packet_debug_bytes <= MAX_DEBUG_PACKET_BYTES);
    }

    fn assert_totals_match_all_hits(
        hits: &VecDeque<Hit>,
        stats: &HashMap<u32, CharacterStats>,
        total_damage: f64,
        total_damage_taken: f64,
        duration: f64,
    ) {
        let expected_damage: f64 = hits
            .iter()
            .filter(|hit| !hit.direction.is_incoming())
            .map(Hit::total_damage)
            .sum();
        let expected_damage_taken: f64 = hits
            .iter()
            .filter(|hit| hit.direction.is_incoming())
            .map(Hit::total_damage)
            .sum();
        assert_eq!(total_damage, expected_damage);
        assert_eq!(total_damage_taken, expected_damage_taken);

        for hit in hits {
            assert!(stats.contains_key(&hit.char_id));
        }
        for (&char_id, row) in stats {
            let char_hits: Vec<_> = hits.iter().filter(|hit| hit.char_id == char_id).collect();
            assert_eq!(
                row.hits,
                char_hits
                    .iter()
                    .filter(|hit| !hit.direction.is_incoming())
                    .count() as u64
            );
            assert_eq!(
                row.damage,
                char_hits
                    .iter()
                    .filter(|hit| !hit.direction.is_incoming())
                    .map(|hit| hit.total_damage())
                    .sum::<f64>()
            );
            assert_eq!(
                row.hits_taken,
                char_hits
                    .iter()
                    .filter(|hit| hit.direction.is_incoming())
                    .count() as u64
            );
            assert_eq!(
                row.damage_taken,
                char_hits
                    .iter()
                    .filter(|hit| hit.direction.is_incoming())
                    .map(|hit| hit.total_damage())
                    .sum::<f64>()
            );
        }

        let outgoing_timestamps: Vec<_> = hits
            .iter()
            .filter(|hit| !hit.direction.is_incoming())
            .map(|hit| hit.timestamp)
            .collect();
        let expected_duration =
            (outgoing_timestamps.last().unwrap() - outgoing_timestamps.first().unwrap()).max(0.001);
        assert_eq!(duration, expected_duration);
        assert_eq!(duration, 149_999.0);
    }

    fn long_combat_hits() -> Vec<Hit> {
        const LONG_COMBAT_HIT_COUNT: usize = 50_001;
        let mut hits = Vec::with_capacity(LONG_COMBAT_HIT_COUNT);
        hits.push(test_hit(-100_000.0, 99, "outgoing", 1_000_000.0));
        for index in 0..LONG_COMBAT_HIT_COUNT - 1 {
            let direction = if index % 3 == 0 {
                "incoming"
            } else {
                "outgoing"
            };
            hits.push(test_hit(
                index as f64,
                (index % 4 + 1) as u32,
                direction,
                (index % 10 + 1) as f64,
            ));
        }
        hits
    }

    #[test]
    fn unknown_hits_remain_output_and_directions_are_summarized_separately() {
        let mut state = CombatState::default();
        state.push_hit(test_hit(1.0, 1, "outgoing", 100.0));
        state.push_hit(test_hit(2.0, 1, "unknown", 40.0));
        state.push_hit(test_hit(3.0, 1, "incoming", 25.0));

        assert_eq!(state.total_damage, 140.0);
        assert_eq!(state.total_damage_taken, 25.0);
        let summary = summarize_hit_directions(&state.hits);
        assert_eq!(summary.outgoing_damage, 100.0);
        assert_eq!(summary.outgoing_hits, 1);
        assert_eq!(summary.unknown_damage, 40.0);
        assert_eq!(summary.unknown_hits, 1);
        assert_eq!(summary.incoming_damage, 25.0);
        assert_eq!(summary.incoming_hits, 1);
        assert!((summary.unknown_share() - 28.571_428_571).abs() < 1e-6);
    }

    #[test]
    fn timeline_handles_empty_hits() {
        let hits = Vec::<Hit>::new();
        let timeline = summarize_timeline(hits.iter(), 1.0);

        assert_eq!(timeline.bucket_seconds, 1.0);
        assert!(timeline.buckets.is_empty());
        assert_eq!(timeline.total_damage, 0.0);
    }

    #[test]
    fn timeline_buckets_damage_by_second_and_role() {
        let mut first = test_hit(10.0, 1, "outgoing", 100.0);
        first.char_name = "一号".to_owned();
        let mut same_bucket = test_hit(10.9, 2, "unknown", 50.0);
        same_bucket.char_name = "二号".to_owned();
        let mut next_bucket = test_hit(11.0, 1, "outgoing", 200.0);
        next_bucket.char_name = "一号".to_owned();
        let incoming = test_hit(11.2, 3, "incoming", 999.0);
        let hits = Vec::from([first, same_bucket, next_bucket, incoming]);

        let timeline = summarize_timeline(hits.iter(), 1.0);

        assert_eq!(timeline.buckets.len(), 2);
        assert_eq!(timeline.total_damage, 350.0);
        assert_eq!(timeline.buckets[0].damage, 150.0);
        assert_eq!(timeline.buckets[0].hits, 2);
        assert_eq!(timeline.buckets[0].role_damage.len(), 2);
        assert_eq!(timeline.buckets[1].damage, 200.0);
        assert_eq!(timeline.buckets[1].cumulative_damage, 350.0);
    }

    #[test]
    fn timeline_extreme_finite_span_saturates_before_allocation() {
        let mut state = CombatState::default();
        state.push_hit(test_hit(-f64::MAX, 1, "outgoing", 100.0));
        state.push_hit(test_hit(0.0, 2, "outgoing", 50.0));
        state.push_hit(test_hit(f64::MAX / 2.0, 3, "outgoing", 75.0));
        state.push_hit(test_hit(f64::MAX, 1, "outgoing", 200.0));

        let timeline = state.timeline_bounded(f64::MIN_POSITIVE, false, 8, 8, 8);
        let legacy = summarize_timeline_with_time_stop(
            state.hits.iter(),
            &state.time_stop,
            state.started_at,
            state.ended_at,
            Vec::new(),
            TimelineAggregationOptions {
                bucket_seconds: f64::MIN_POSITIVE,
                subtract_time_stop: false,
                max_buckets: 8,
                max_roles_per_bucket: 8,
                max_characters: 8,
            },
        );

        assert_eq!(timeline, legacy);
        assert_eq!(timeline.buckets.len(), 8);
        assert!(timeline.bucket_seconds.is_finite());
        assert!(timeline.bucket_seconds > 0.0);
        assert!(
            timeline
                .buckets
                .iter()
                .all(|bucket| { bucket.start_offset.is_finite() && bucket.end_offset.is_finite() })
        );
        assert_eq!(timeline.total_damage, 425.0);
        assert_eq!(
            timeline
                .buckets
                .iter()
                .map(|bucket| bucket.hits)
                .sum::<u64>(),
            4
        );
    }

    #[test]
    fn timeline_enforces_source_role_and_character_budgets_without_losing_team_totals() {
        let mut state = CombatState::default();
        for char_id in 1..=10 {
            state.push_hit(test_hit(10.0, char_id, "outgoing", 10.0));
            state.push_hit(test_hit(11.0, char_id, "outgoing", 10.0));
        }

        // Zero budgets normalize to one; no caller can trigger an empty-vector
        // index or bypass source-side bounds.
        let minimum = state.timeline_bounded(1.0, false, 0, 0, 0);
        assert_eq!(minimum.buckets.len(), 1);
        assert!(minimum.buckets[0].role_damage.len() <= 1);
        assert_eq!(minimum.total_damage, 200.0);
        assert_eq!(minimum.buckets[0].hits, 20);

        let timeline = state.timeline_bounded(1.0, false, 4, 2, 3);
        assert!(timeline.buckets.len() <= 4);
        assert!(
            timeline
                .buckets
                .iter()
                .all(|bucket| bucket.role_damage.len() <= 2)
        );
        let retained_characters = timeline
            .buckets
            .iter()
            .flat_map(|bucket| bucket.role_damage.iter().map(|role| role.char_id))
            .collect::<HashSet<_>>();
        assert!(retained_characters.len() <= 3);
        assert_eq!(timeline.total_damage, 200.0);
        assert_eq!(
            timeline
                .buckets
                .iter()
                .map(|bucket| bucket.hits)
                .sum::<u64>(),
            20
        );
        assert!(timeline.omitted_role_damage > 0.0);
        assert!(timeline.omitted_role_hits > 0);
    }

    #[test]
    fn compact_timeline_role_index_and_omission_are_source_bounded() {
        const DISTINCT_CHARACTERS: u32 = 1_024;
        let mut state = CombatState::default();
        for char_id in 0..DISTINCT_CHARACTERS {
            state.push_hit(test_hit(1.0, char_id, "outgoing", 1.0));
        }

        assert_eq!(
            state.compact_timeline.roles.len(),
            COMPACT_TIMELINE_MAX_CHARACTERS
        );
        assert_eq!(
            state.bounded_timeline_character_count(),
            BoundedTimelineCharacterCount {
                retained: COMPACT_TIMELINE_MAX_CHARACTERS,
                overflowed: true,
            }
        );
        let timeline = state.timeline_bounded(1.0, false, 1, usize::MAX, usize::MAX);
        assert_eq!(timeline.total_damage, f64::from(DISTINCT_CHARACTERS));
        assert_eq!(timeline.buckets[0].hits, u64::from(DISTINCT_CHARACTERS));
        assert_eq!(
            timeline.buckets[0].role_damage.len(),
            COMPACT_TIMELINE_MAX_CHARACTERS
        );
        assert_eq!(
            timeline.omitted_role_hits,
            u64::from(DISTINCT_CHARACTERS) - COMPACT_TIMELINE_MAX_CHARACTERS as u64
        );
        assert_eq!(
            timeline.omitted_role_damage,
            f64::from(DISTINCT_CHARACTERS) - COMPACT_TIMELINE_MAX_CHARACTERS as f64
        );
    }

    #[test]
    fn compact_timeline_dense_projection_bounds_candidates_to_bucket_role_budget() {
        const ROLE_COUNT: u32 = 256;
        const BUCKET_COUNT: usize = 10_000;
        const ROLES_PER_BUCKET: usize = 16;
        const CANDIDATE_LIMIT: usize = BUCKET_COUNT * ROLES_PER_BUCKET;

        // Build one real 10k-leaf timestamp tree, then clone its arena for each
        // role. This creates the same dense 256 x 10k projection surface as
        // 2.56 million pushes without spending the fixture setup on Hit DTOs,
        // aggregate maps, mutation records, and repeated crit-bit insertion.
        let mut template = CompactTimelineTree::default();
        for bucket in 0..BUCKET_COUNT {
            template.adjust(bucket as f64, 1.0, 1, bucket as u64);
        }

        let mut roles = HashMap::with_capacity(ROLE_COUNT as usize);
        for char_id in 0..ROLE_COUNT {
            let mut tree = template.clone();
            scale_dense_timeline_tree(&mut tree, 1.0, 1, u64::from(ROLE_COUNT), u64::from(char_id));
            roles.insert(
                char_id,
                CompactTimelineRoleIndex {
                    name: format!("role-{char_id:03}"),
                    tree,
                },
            );
        }
        let mut team = template;
        scale_dense_timeline_tree(
            &mut team,
            f64::from(ROLE_COUNT),
            u64::from(ROLE_COUNT),
            u64::from(ROLE_COUNT),
            0,
        );
        let index = CompactTimelineIndex {
            team,
            active_roles: roles.len(),
            roles,
            role_overflowed: false,
            start: Some(0.0),
            end: Some((BUCKET_COUNT - 1) as f64),
            next_sequence: (BUCKET_COUNT as u64).saturating_mul(u64::from(ROLE_COUNT)),
        };

        reset_compact_timeline_role_scratch_slots();
        reset_compact_timeline_role_candidate_slots();
        let timeline = index
            .project(1.0, BUCKET_COUNT, ROLES_PER_BUCKET, ROLE_COUNT as usize)
            .expect("dense timeline projection");
        let output_roles = timeline
            .buckets
            .iter()
            .flat_map(|bucket| &bucket.roles)
            .count();
        let total_damage = timeline
            .buckets
            .iter()
            .map(|bucket| bucket.damage)
            .sum::<f64>();
        let total_hits = timeline
            .buckets
            .iter()
            .map(|bucket| bucket.hits)
            .sum::<u64>();
        let dense_event_count = BUCKET_COUNT as u64 * u64::from(ROLE_COUNT);

        assert_eq!(timeline.buckets.len(), BUCKET_COUNT);
        assert_eq!(total_damage, dense_event_count as f64);
        assert_eq!(total_hits, dense_event_count);
        assert_eq!(
            timeline.omitted_role_damage,
            (dense_event_count as usize - CANDIDATE_LIMIT) as f64
        );
        assert_eq!(
            timeline.omitted_role_hits,
            dense_event_count - CANDIDATE_LIMIT as u64
        );
        assert_eq!(
            compact_timeline_role_scratch_slots(),
            BUCKET_COUNT,
            "role projection must initialize one reusable bucket scratch, not one per role"
        );
        let candidate_slots = compact_timeline_role_candidate_slots();
        assert!(
            candidate_slots <= CANDIDATE_LIMIT,
            "candidate slots must stay within bucket_count * max_roles_per_bucket"
        );
        assert_eq!(
            candidate_slots, CANDIDATE_LIMIT,
            "the dense fixture must fill every bounded candidate slot"
        );
        assert!(
            output_roles <= CANDIDATE_LIMIT,
            "output roles must stay within bucket_count * max_roles_per_bucket"
        );
        assert_eq!(output_roles, CANDIDATE_LIMIT);
        assert!(
            timeline.buckets.iter().all(|bucket| {
                bucket.roles.len() == ROLES_PER_BUCKET
                    && bucket.roles.iter().all(|role| role.char_id < 16)
            }),
            "each dense bucket retains the deterministic earliest 16 roles"
        );
    }

    #[test]
    fn indexed_timeline_selects_global_earliest_characters_then_bucket_earliest_roles() {
        let mut state = CombatState::default();
        for (timestamp, char_id, damage) in [
            (0.0, 1, 1.0),
            (0.0, 2, 2.0),
            (0.0, 3, 3.0),
            (1.0, 3, 4.0),
            (1.0, 2, 5.0),
        ] {
            let mut hit = test_hit(timestamp, char_id, "outgoing", damage);
            hit.char_name = format!("role-{char_id}");
            state.push_hit(hit);
        }
        let indexed = state.timeline_bounded(1.0, false, 2, 1, 2);

        assert_eq!(indexed.buckets[0].role_damage[0].char_id, 1);
        assert_eq!(indexed.buckets[1].role_damage[0].char_id, 2);
        assert_eq!(indexed.total_damage, 15.0);
        assert_eq!(
            indexed
                .buckets
                .iter()
                .map(|bucket| bucket.hits)
                .sum::<u64>(),
            5
        );
        assert_eq!(indexed.omitted_role_damage, 9.0);
        assert_eq!(indexed.omitted_role_hits, 3);
    }

    #[test]
    fn indexed_timeline_matches_legacy_scan_after_mutations_and_clock_extension() {
        let mut state = CombatState::default();
        for index in 0..24_u32 {
            let timestamp = ((index * 7) % 24) as f64 * 0.5;
            let direction = if index % 7 == 0 {
                "incoming"
            } else {
                "outgoing"
            };
            let mut hit = test_hit(timestamp, index % 5 + 1, direction, f64::from(index + 1));
            hit.char_name = format!("role-{}", hit.char_id);
            state.push_hit(hit);
        }

        let mut source = test_hit(12.0, 2, "outgoing", 100.0);
        source.char_name = "role-2".to_owned();
        source.target_hp_before = 1_000.0;
        source.target_hp_after = 900.0;
        source.target_max_hp = 1_000.0;
        source.gameplay_effect_index = Some(42);
        state.push_hit(source);
        assert!(state.apply_damage_correction(HitDamageCorrection {
            source_timestamp: 12.0,
            source_char_id: 2,
            source_damage: 100.0,
            source_target_hp_before: 1_000.0,
            source_target_hp_after: 900.0,
            source_target_max_hp: 1_000.0,
            source_gameplay_effect_index: Some(42),
            damage: 150.0,
            target_hp_before: 1_050.0,
            target_hp_after: 900.0,
            target_hp_percent: 90.0,
            max_hp_reduction: None,
            reconciled_overkill_damage: None,
        }));
        assert!(state.apply_follow_up(HitFollowUp {
            source_timestamp: 12.0,
            source_char_id: 2,
            source_damage: 100.0,
            source_target_hp_before: 1_000.0,
            source_target_hp_after: 900.0,
            source_target_max_hp: 1_000.0,
            source_gameplay_effect_index: Some(42),
            timestamp: 12.25,
            damage: 25.0,
            target_hp_after: 875.0,
            target_hp_percent: 87.5,
            damage_name: Some("覆纹追加攻击".to_owned()),
            attack_type: Some("覆纹".to_owned()),
            damage_attribute: Some("灵".to_owned()),
        }));
        assert_eq!(
            state.skill_breakdown(None),
            summarize_skill_breakdown(&state.hits, None),
            "push, correction, and follow-up deltas must preserve skill aggregation parity"
        );
        assert_eq!(
            state.skill_breakdown_index.directions,
            summarize_hit_directions(&state.hits),
            "diagnostic direction aggregates must match the authoritative hits"
        );
        // Combat clock semantics include a pause transition after the final
        // hit. The indexed projection must retain that trailing empty range.
        apply_test_pause(&mut state, 13.0, 15.0);

        let options = TimelineAggregationOptions {
            bucket_seconds: 1.0,
            subtract_time_stop: true,
            max_buckets: 7,
            max_roles_per_bucket: 2,
            max_characters: 5,
        };
        let legacy = summarize_timeline_with_time_stop(
            state.hits.iter(),
            &state.time_stop,
            state.started_at,
            state.ended_at,
            Vec::new(),
            options,
        );
        let indexed = state.timeline_bounded(
            options.bucket_seconds,
            options.subtract_time_stop,
            options.max_buckets,
            options.max_roles_per_bucket,
            options.max_characters,
        );

        assert_eq!(indexed, legacy);
        assert_eq!(indexed.end_timestamp, Some(15.0));
        assert_eq!(indexed.buckets.len(), 7);
    }

    #[test]
    fn indexed_timeline_projection_visits_bounded_paths_for_large_history() {
        const HIT_COUNT: usize = 50_001;
        const OUTPUT_BUCKETS: usize = 60;
        let mut state = CombatState::default();
        for index in 0..HIT_COUNT {
            // The timestamp bits are entirely replay-controlled. Alternating
            // fractional offsets exercises nontrivial crit-bit insertion order
            // without relying on a balanced/randomized tree.
            let timestamp = (index / 2) as f64 + if index % 2 == 0 { 0.875 } else { 0.125 };
            state.push_hit(test_hit(timestamp, (index % 4 + 1) as u32, "outgoing", 1.0));
        }

        assert!(state.compact_timeline.team.max_depth() <= COMPACT_TIMELINE_MAX_TREE_DEPTH);
        assert!(
            state
                .compact_timeline
                .roles
                .values()
                .all(|role| role.tree.max_depth() <= COMPACT_TIMELINE_MAX_TREE_DEPTH)
        );
        reset_compact_timeline_projection_visits();
        let timeline = state.timeline_bounded(0.2, false, OUTPUT_BUCKETS, 4, 4);
        let visits = compact_timeline_projection_visits();

        assert_eq!(timeline.buckets.len(), OUTPUT_BUCKETS);
        assert_eq!(timeline.total_damage, HIT_COUNT as f64);
        assert_eq!(
            timeline
                .buckets
                .iter()
                .map(|bucket| bucket.hits)
                .sum::<u64>(),
            HIT_COUNT as u64
        );
        let structural_visit_bound =
            (4 + 1) * (1 + 2 * OUTPUT_BUCKETS * COMPACT_TIMELINE_MAX_TREE_DEPTH);
        assert!(
            visits <= structural_visit_bound,
            "projection visited {visits} nodes, exceeding the bucket/depth bound {structural_visit_bound}"
        );
        assert!(
            visits < HIT_COUNT / 2,
            "bounded projection visited {visits} index nodes for {HIT_COUNT} retained hits"
        );
    }

    #[test]
    fn indexed_skill_projection_visits_distinct_rows_instead_of_retained_hits() {
        const HIT_COUNT: usize = 50_001;
        let mut state = CombatState::default();
        for index in 0..HIT_COUNT {
            state.push_hit(test_hit(index as f64, 1, "outgoing", 1.0));
        }

        reset_skill_index_projection_visits();
        let breakdown = state.skill_breakdown(None);
        let visits = skill_index_projection_visits();

        assert_eq!(state.hits.len(), HIT_COUNT);
        assert_eq!(breakdown.total_hits, HIT_COUNT as u64);
        assert_eq!(breakdown.total_damage, HIT_COUNT as f64);
        assert_eq!(breakdown.rows.len(), 1);
        assert_eq!(visits, 1);

        reset_combat_detail_query_visits();
        let middle = state.indexed_combat_details(
            None,
            &IndexedCombatDetailFilter::All,
            None,
            HIT_COUNT / 2,
            2,
        );
        assert_eq!(middle.rows.len(), 2);
        assert_eq!(middle.rows[0].0, HIT_COUNT / 2);
        assert_eq!(combat_detail_query_visits(), 2);

        reset_combat_detail_query_visits();
        let filtered_middle = state.indexed_combat_details(
            None,
            &IndexedCombatDetailFilter::Outgoing,
            None,
            HIT_COUNT / 2,
            2,
        );
        assert_eq!(filtered_middle.rows.len(), 2);
        assert_eq!(filtered_middle.rows[0].0, HIT_COUNT / 2);
        assert_eq!(combat_detail_query_visits(), 2);

        reset_combat_detail_query_visits();
        let skill = hit_skill_id_ref(state.hits.front().expect("large fixture hit"));
        let page = state.indexed_combat_details(
            Some(1),
            &IndexedCombatDetailFilter::Outgoing,
            Some(skill),
            HIT_COUNT - 2,
            2,
        );
        assert_eq!(page.total_hits, HIT_COUNT);
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[0].0, HIT_COUNT - 2);
        assert_eq!(page.rows[1].0, HIT_COUNT - 1);
        assert_eq!(combat_detail_query_visits(), 2);

        reset_combat_detail_query_visits();
        let empty = state.indexed_combat_details(
            Some(1),
            &IndexedCombatDetailFilter::Outgoing,
            Some(skill),
            usize::MAX,
            2,
        );
        assert!(empty.rows.is_empty());
        assert_eq!(combat_detail_query_visits(), 0);
    }

    #[test]
    fn combat_stats_and_hot_readouts_are_bounded_for_many_character_ids() {
        const CHARACTER_COUNT: usize = 50_000;
        let mut state = CombatState::default();
        for index in 0..CHARACTER_COUNT {
            state.push_hit(test_hit(index as f64, index as u32, "outgoing", 1.0));
        }

        assert_eq!(state.hits.len(), CHARACTER_COUNT);
        assert_eq!(state.total_damage, CHARACTER_COUNT as f64);
        assert!(state.stats.len() <= MAX_COMBAT_STAT_ROWS);
        assert!(
            state
                .stats
                .contains_key(&COMBAT_STATS_OVERFLOW_CHARACTER_ID)
        );
        assert_eq!(
            state.stats.values().map(|row| row.hits).sum::<u64>(),
            CHARACTER_COUNT as u64
        );
        assert_eq!(
            state.stats.values().map(|row| row.damage).sum::<f64>(),
            CHARACTER_COUNT as f64
        );
        let attribution = state.damage_attribution_summary();
        assert_eq!(attribution.total_damage, CHARACTER_COUNT as f64);
        assert_eq!(attribution.character_direct_damage, CHARACTER_COUNT as f64);
        assert_eq!(attribution.unattributed_damage, 0.0);
    }

    #[test]
    fn combat_detail_position_index_has_a_constant_slot_factor() {
        const HIT_COUNT: usize = 1_000_000;
        let hit = test_hit(1.0, 7, "incoming", 1.0);
        let mut index = CombatDetailIndex::default();
        for position in 0..HIT_COUNT {
            index.observe_hit(position, &hit);
        }

        // One character list, one skill list, and one incoming-filter list.
        // No scope/filter/skill Cartesian product duplicates positions.
        assert_eq!(index.position_slot_count(), HIT_COUNT * 3);
        assert!(index.buckets.len() <= MAX_INDEXED_DETAIL_AGGREGATES);
    }

    #[test]
    fn bulk_history_rehydrate_rebuilds_all_indexes_in_one_hit_pass() {
        const HIT_COUNT: usize = 50_001;
        let hit = test_hit(1.0, 7, "outgoing", 1.0);
        let mut state = CombatState::default();

        reset_bulk_combat_rebuild_visits();
        state.replace_global_hits_bulk(std::iter::repeat_n(hit, HIT_COUNT).collect());

        assert_eq!(bulk_combat_rebuild_visits(), HIT_COUNT);
        assert_eq!(state.hits.len(), HIT_COUNT);
        assert_eq!(state.total_damage, HIT_COUNT as f64);
        assert_eq!(state.stats[&7].hits, HIT_COUNT as u64);
        assert_eq!(state.skill_breakdown(None).total_hits, HIT_COUNT as u64);
        let detail = state.indexed_combat_details(
            None,
            &IndexedCombatDetailFilter::All,
            None,
            HIT_COUNT / 2,
            2,
        );
        assert_eq!(detail.total_hits, HIT_COUNT);
        assert_eq!(detail.rows.len(), 2);
    }

    #[test]
    fn bulk_history_rehydrate_matches_incremental_direction_and_attribution_totals() {
        let mut outgoing = test_hit(1.0, 7, "outgoing", 100.0);
        outgoing.attack_type = Some("Normal Attack".to_owned());
        outgoing.max_hp_reduction = 123.0;
        let unknown = test_hit(2.0, 7, "unknown", 50.0);
        let incoming = test_hit(3.0, 7, "incoming", 25.0);
        let hits = vec![outgoing, unknown, incoming];
        let mut incremental = CombatState::default();
        for hit in hits.iter().cloned() {
            incremental.push_hit(hit);
        }
        let mut bulk = CombatState::default();
        bulk.replace_global_hits_bulk(hits);

        assert_eq!(bulk.total_damage, incremental.total_damage);
        assert_eq!(bulk.total_damage_taken, incremental.total_damage_taken);
        assert_eq!(bulk.max_hp_reduction, 123.0);
        assert_eq!(bulk.started_at, incremental.started_at);
        assert_eq!(bulk.ended_at, incremental.ended_at);
        let bulk_stats = bulk.stats.get(&7).expect("bulk character stats");
        let incremental_stats = incremental
            .stats
            .get(&7)
            .expect("incremental character stats");
        assert_eq!(bulk_stats.hits, incremental_stats.hits);
        assert_eq!(bulk_stats.damage, incremental_stats.damage);
        assert_eq!(bulk_stats.hits_taken, incremental_stats.hits_taken);
        assert_eq!(bulk_stats.damage_taken, incremental_stats.damage_taken);
        assert_eq!(
            bulk_stats.attributed_hits,
            incremental_stats.attributed_hits
        );
        assert_eq!(
            bulk_stats.attributed_damage,
            incremental_stats.attributed_damage
        );
        assert_eq!(
            bulk.skill_breakdown(None),
            incremental.skill_breakdown(None)
        );
        assert_eq!(
            bulk.damage_attribution_summary(),
            incremental.damage_attribution_summary()
        );
    }

    #[test]
    fn indexed_combat_detail_page_matches_mutated_authoritative_hits() {
        let mut state = CombatState::default();
        let mut first = test_hit(1.0, 7, "outgoing", 100.0);
        first.damage_name = Some("Skill A".to_owned());
        first.gameplay_effect_index = Some(17);
        first.target_hp_before = 1_000.0;
        first.target_hp_after = 900.0;
        first.target_max_hp = 1_000.0;
        state.push_hit(first);
        let mut second = test_hit(2.0, 7, "unknown", 50.0);
        second.damage_name = Some("Skill B".to_owned());
        state.push_hit(second);
        state.push_hit(test_hit(3.0, 8, "incoming", 25.0));

        assert!(state.apply_damage_correction(HitDamageCorrection {
            source_timestamp: 1.0,
            source_char_id: 7,
            source_damage: 100.0,
            source_target_hp_before: 1_000.0,
            source_target_hp_after: 900.0,
            source_target_max_hp: 1_000.0,
            source_gameplay_effect_index: Some(17),
            damage: 125.0,
            target_hp_before: 1_025.0,
            target_hp_after: 900.0,
            target_hp_percent: 90.0,
            max_hp_reduction: Some(250.0),
            reconciled_overkill_damage: None,
        }));
        let attribution = state.damage_attribution_summary();
        assert_eq!(attribution.total_damage, 175.0);
        assert_eq!(attribution.max_hp_reduction, 250.0);
        assert_eq!(
            attribution
                .with_max_hp_reduction_in_total(true)
                .total_damage,
            425.0
        );
        assert!(state.apply_follow_up(HitFollowUp {
            source_timestamp: 1.0,
            source_char_id: 7,
            source_damage: 100.0,
            source_target_hp_before: 1_000.0,
            source_target_hp_after: 900.0,
            source_target_max_hp: 1_000.0,
            source_gameplay_effect_index: Some(17),
            timestamp: 1.25,
            damage: 25.0,
            target_hp_after: 875.0,
            target_hp_percent: 87.5,
            damage_name: Some("Follow-up".to_owned()),
            attack_type: Some("覆纹".to_owned()),
            damage_attribute: None,
        }));

        let page =
            state.indexed_combat_details(Some(7), &IndexedCombatDetailFilter::All, None, 1, 1);
        let expected = state
            .hits
            .iter()
            .filter(|hit| hit.char_id == 7)
            .collect::<Vec<_>>();
        assert_eq!(page.total_hits, expected.len());
        assert_eq!(
            page.total_damage,
            expected.iter().map(|hit| hit.total_damage()).sum::<f64>()
        );
        assert_eq!(page.rows.len(), 1);
        assert!(std::ptr::eq(page.rows[0].1, expected[1]));
        assert_eq!(page.directions.outgoing_hits, 1);
        assert_eq!(page.directions.unknown_hits, 1);

        let skill_page = state.indexed_combat_details(
            Some(7),
            &IndexedCombatDetailFilter::Outgoing,
            Some("Skill A"),
            0,
            10,
        );
        assert_eq!(skill_page.total_hits, 1);
        assert_eq!(skill_page.total_damage, 150.0);
        assert_eq!(skill_page.rows.len(), 1);
        assert_eq!(skill_page.rows[0].1.max_hp_reduction, 250.0);
    }

    #[test]
    fn timeline_marks_time_stop_without_inflating_bucket_dps() {
        let mut state = CombatState::default();
        state.push_hit(test_hit(0.0, 1, "outgoing", 100.0));
        apply_test_pause(&mut state, 0.25, 0.75);
        state.push_hit(test_hit(1.0, 1, "outgoing", 100.0));

        let timeline = state.timeline(1.0, true);

        assert_eq!(timeline.time_stop_intervals.len(), 1);
        assert!((timeline.time_stop_intervals[0].start_offset - 0.25).abs() < 1e-9);
        assert!((timeline.time_stop_intervals[0].end_offset - 0.75).abs() < 1e-9);
        assert!((timeline.buckets[0].dps - 100.0).abs() < 1e-9);
    }

    #[test]
    fn timeline_clamps_abyss_markers_to_chart_edges() {
        let mut state = CombatState::default();
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 0.0,
            cycle: Some(1),
            floor: Some(1),
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });
        state.push_hit(test_hit(1.0, 1, "outgoing", 100.0));
        state.push_hit(test_hit(2.0, 1, "outgoing", 100.0));
        state.apply_abyss_event(AbyssEvent::Success { timestamp: 3.0 });
        state.apply_abyss_event(AbyssEvent::Exit { timestamp: 4.0 });

        let timeline = state.timeline(1.0, false);

        assert_eq!(timeline.start_timestamp, Some(1.0));
        assert_eq!(timeline.end_timestamp, Some(2.0));
        assert!(timeline.markers.iter().any(|marker| {
            marker.label == "Ascending Line"
                && marker.kind == TimelineMarkerKind::HalfStart
                && marker.offset == 0.0
        }));
        assert!(timeline.markers.iter().any(|marker| {
            marker.label == "Cleared"
                && marker.kind == TimelineMarkerKind::Clear
                && (marker.offset - 1.0).abs() < 1e-9
        }));
        assert!(timeline.markers.iter().any(|marker| {
            marker.label == "Left"
                && marker.kind == TimelineMarkerKind::Exit
                && (marker.offset - 1.0).abs() < 1e-9
        }));
    }

    #[test]
    fn abyss_half_timeline_can_include_run_markers() {
        let mut state = CombatState::default();
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 0.0,
            cycle: Some(1),
            floor: Some(1),
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });
        state.push_hit(test_hit(1.0, 1, "outgoing", 100.0));
        state.push_hit(test_hit(2.0, 1, "outgoing", 100.0));
        state.apply_abyss_event(AbyssEvent::Success { timestamp: 3.0 });
        state.apply_abyss_event(AbyssEvent::Exit { timestamp: 4.0 });

        let mut timeline = state.abyss.first_half.timeline(1.0, false);
        if let (Some(start), Some(end)) = (timeline.start_timestamp, timeline.end_timestamp) {
            timeline.markers = state
                .abyss
                .timeline_markers_for_half(AbyssHalf::First, start, end);
        }

        assert_eq!(timeline.start_timestamp, Some(1.0));
        assert_eq!(timeline.end_timestamp, Some(2.0));
        assert!(timeline.markers.iter().any(|marker| {
            marker.label == "Ascending Line"
                && marker.kind == TimelineMarkerKind::HalfStart
                && marker.offset == 0.0
        }));
        assert!(timeline.markers.iter().any(|marker| {
            marker.label == "Cleared"
                && marker.kind == TimelineMarkerKind::Clear
                && (marker.offset - 1.0).abs() < 1e-9
        }));
        assert!(timeline.markers.iter().any(|marker| {
            marker.label == "Left"
                && marker.kind == TimelineMarkerKind::Exit
                && (marker.offset - 1.0).abs() < 1e-9
        }));
    }

    #[test]
    fn skill_breakdown_splits_follow_up_and_unknown_attribution() {
        let mut source = test_hit(1.0, 10, "outgoing", 100.0);
        source.char_name = "主输出".to_owned();
        source.attack_type = Some("普攻".to_owned());
        source.damage_name = Some("普攻一段".to_owned());
        source.ability_name = Some("GA_Test_Melee".to_owned());
        source.follow_up_damage = 25.0;
        source.follow_up_damage_name = Some("覆纹追加".to_owned());
        source.follow_up_attack_type = Some("覆纹".to_owned());

        let mut unknown = test_hit(2.0, 99, "unknown", 50.0);
        unknown.char_known = false;
        unknown.char_name = "未知角色".to_owned();
        unknown.gameplay_effect_index = Some(777);

        let hits = Vec::from([source, unknown]);
        let breakdown = summarize_skill_breakdown(hits.iter(), None);

        assert_eq!(breakdown.total_damage, 175.0);
        assert_eq!(breakdown.total_hits, 3);
        assert_eq!(breakdown.rows.len(), 3);
        assert!(
            breakdown
                .rows
                .iter()
                .any(|row| row.name == "覆纹追加" && row.is_follow_up)
        );
        assert_eq!(breakdown.unknown.unknown_character_count, 1);
        assert_eq!(breakdown.unknown.unknown_direction_hits, 1);
        assert_eq!(breakdown.unknown.unmapped_skill_hits, 1);
        assert_eq!(breakdown.unknown.unmapped_gameplay_effects[0].index, 777);
    }

    #[test]
    fn skill_breakdown_merges_same_ability_across_effects() {
        let mut first = test_hit(1.0, 10, "outgoing", 100.0);
        first.attack_type = Some("Q技能".to_owned());
        first.ability_name = Some("GA_Test_UltraSkill".to_owned());
        first.damage_name = Some("Test Ultimate".to_owned());
        first.gameplay_effect_index = Some(101);
        first.gameplay_effect_name = Some("GE_Test_UltraSkill1_Damage".to_owned());

        let mut second = test_hit(2.0, 10, "outgoing", 75.0);
        second.attack_type = Some("Q技能".to_owned());
        second.ability_name = Some("GA_Test_UltraSkill".to_owned());
        second.damage_name = Some("测试大招".to_owned());
        second.gameplay_effect_index = Some(102);
        second.gameplay_effect_name = Some("GE_Test_UltraSkill2_Damage".to_owned());

        let hits = Vec::from([first, second]);
        let breakdown = summarize_skill_breakdown(hits.iter(), None);

        assert_eq!(breakdown.rows.len(), 1);
        assert_eq!(breakdown.rows[0].name, "GA_Test_UltraSkill");
        assert_eq!(breakdown.rows[0].hits, 2);
        assert_eq!(breakdown.rows[0].damage, 175.0);
        assert_eq!(
            breakdown.rows[0].ability_name.as_deref(),
            Some("GA_Test_UltraSkill")
        );
        assert!(breakdown.rows[0].gameplay_effect_index.is_none());
        assert!(breakdown.rows[0].gameplay_effect_name.is_none());
    }

    #[test]
    fn skill_breakdown_splits_exact_semantic_components_under_one_ability() {
        let mut first = test_hit(1.0, 10, "outgoing", 100.0);
        first.attack_type = Some("普攻".to_owned());
        first.ability_name = Some("GA_Test_Melee".to_owned());
        first.damage_component = Some("Fang Thrust (1 Stack)".to_owned());
        first.gameplay_effect_index = Some(101);
        first.gameplay_effect_name = Some("GE_Test_ShadowAtk_Damage".to_owned());

        let mut second = test_hit(2.0, 10, "outgoing", 75.0);
        second.attack_type = Some("普攻".to_owned());
        second.ability_name = Some("GA_Test_Melee".to_owned());
        second.damage_component = Some("Fang Thrust (2 Stacks)".to_owned());
        second.gameplay_effect_index = Some(102);
        second.gameplay_effect_name = Some("GE_Test_ShadowAtk1_Damage".to_owned());

        let hits = Vec::from([first, second]);
        let breakdown = summarize_skill_breakdown(hits.iter(), None);

        assert_eq!(breakdown.rows.len(), 2);
        assert!(breakdown.rows.iter().any(|row| {
            row.name == "Fang Thrust (1 Stack)"
                && row.damage_name.as_deref() == Some("Fang Thrust (1 Stack)")
                && row.damage == 100.0
        }));
        assert!(breakdown.rows.iter().any(|row| {
            row.name == "Fang Thrust (2 Stacks)"
                && row.damage_name.as_deref() == Some("Fang Thrust (2 Stacks)")
                && row.damage == 75.0
        }));
    }

    #[test]
    fn skill_breakdown_preserves_shared_effect_identity() {
        let mut first = test_hit(1.0, 10, "outgoing", 100.0);
        first.attack_type = Some("Q技能".to_owned());
        first.ability_name = Some("GA_Test_UltraSkill".to_owned());
        first.gameplay_effect_index = Some(101);
        first.gameplay_effect_name = Some("GE_Test_UltraSkill_Damage".to_owned());
        let mut second = first.clone();
        second.timestamp = 2.0;
        second.damage = 75.0;

        let hits = Vec::from([first, second]);
        let breakdown = summarize_skill_breakdown(hits.iter(), None);

        assert_eq!(breakdown.rows.len(), 1);
        assert_eq!(breakdown.rows[0].gameplay_effect_index, Some(101));
        assert_eq!(
            breakdown.rows[0].gameplay_effect_name.as_deref(),
            Some("GE_Test_UltraSkill_Damage")
        );
    }

    #[test]
    fn capture_quality_summary_is_redacted() {
        let mut state = CombatState::default();
        state.push_hit(test_hit(1.0, 1, "outgoing", 100.0));
        state.observe_packet(PacketObservation { parsed_hits: 1 });
        state.push_packet(PacketDebug {
            timestamp: 1.0,
            source: "192.0.2.1:1111".to_owned(),
            destination: "198.51.100.1:2222".to_owned(),
            direction: "outgoing".to_owned(),
            payload_len: 128,
            declared_ids: vec![1],
            parsed_hits: 1,
            note: "sensitive note".to_owned(),
            payload_preview: "preview text".to_owned(),
            payload_hex: "deadbeef".to_owned(),
            decoded_text: "decoded text".to_owned(),
        });
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 1.0,
            cycle: None,
            floor: Some(1),
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });

        let summary = state.capture_quality_summary(CaptureQualitySource::PcapngReplay);
        let text = summary.redacted_text();

        assert_eq!(summary.packet_count, 1);
        assert_eq!(summary.packets_with_hits, 1);
        assert_eq!(summary.abyss_event_count, 1);
        assert!(text.contains("PCAPNG 回放"));
        assert!(!text.contains("deadbeef"));
        assert!(!text.contains("192.0.2.1"));
        assert!(!text.contains("decoded text"));
    }

    #[test]
    #[cfg(feature = "desktop")]
    fn allocation_free_quality_scalars_match_legacy_out_of_order_time_stops() {
        let mut state = CombatState::default();
        state.push_hit(test_hit(0.0, 1, "outgoing", 1.0));
        state.push_hit(test_hit(30.0, 1, "outgoing", 1.0));
        for event in [
            TimeStopEvent::GamePauseStarted {
                timestamp: 10.0,
                pause_type_mask: 1,
            },
            TimeStopEvent::GamePauseEnded {
                timestamp: 20.0,
                pause_type_mask: 1,
            },
            TimeStopEvent::GamePauseStarted {
                timestamp: 1.0,
                pause_type_mask: 1,
            },
            TimeStopEvent::GamePauseEnded {
                timestamp: 5.0,
                pause_type_mask: 1,
            },
            TimeStopEvent::GamePauseStarted {
                timestamp: 4.0,
                pause_type_mask: 1,
            },
            TimeStopEvent::GamePauseEnded {
                timestamp: 12.0,
                pause_type_mask: 1,
            },
        ] {
            state.apply_time_stop_event(event);
        }

        let legacy = state.capture_quality_summary(CaptureQualitySource::Live);
        let scalars = state.capture_quality_scalars();

        assert_eq!(scalars.hits_generation, state.hits_generation);
        assert_eq!(scalars.hit_count, legacy.hit_count);
        assert_eq!(scalars.packet_count, legacy.packet_count);
        assert_eq!(scalars.packets_with_hits, legacy.packets_with_hits);
        assert_eq!(scalars.time_stop_event_count, legacy.time_stop_event_count);
        assert_eq!(
            scalars.time_stop_interval_count,
            legacy.time_stop_interval_count
        );
        assert_eq!(scalars.abyss_event_count, legacy.abyss_event_count);
        assert_eq!(
            scalars.server_damage_corrections,
            legacy.server_damage_corrections
        );
        assert_eq!(scalars.time_stop_event_count, 3);
        assert_eq!(scalars.time_stop_interval_count, 1);
    }

    #[test]
    #[cfg(feature = "desktop")]
    fn allocation_free_interval_count_matches_materialized_union_cases() {
        let trackers = [
            TimeStopTracker::default(),
            TimeStopTracker {
                intervals: vec![
                    TimeStopInterval {
                        start: 1.0,
                        end: 3.0,
                    },
                    TimeStopInterval {
                        start: 3.0,
                        end: 4.0,
                    },
                    TimeStopInterval {
                        start: 8.0,
                        end: 9.0,
                    },
                ]
                .into(),
                active_game_pause: Some((10.0, 1)),
                ..TimeStopTracker::default()
            },
            TimeStopTracker {
                intervals: vec![
                    TimeStopInterval {
                        start: 10.0,
                        end: 20.0,
                    },
                    TimeStopInterval {
                        start: 1.0,
                        end: 5.0,
                    },
                    TimeStopInterval {
                        start: 4.0,
                        end: 12.0,
                    },
                    TimeStopInterval {
                        start: 30.0,
                        end: 40.0,
                    },
                ]
                .into(),
                active_game_pause: Some((39.0, 1)),
                ..TimeStopTracker::default()
            },
        ];
        for (case, tracker) in trackers.iter().enumerate() {
            for (start, end) in [(0.0, 50.0), (2.0, 11.0), (11.0, 35.0), (5.0, 5.0)] {
                assert_eq!(
                    tracker.interval_count_between(start, end),
                    tracker.intervals_between(start, end).len(),
                    "case {case}, window {start}..{end}"
                );
            }
        }
    }

    #[test]
    fn combat_session_summary_contains_redacted_aggregates() {
        let mut state = CombatState::default();
        let mut hit = test_hit(1.0, 10, "outgoing", 120.0);
        hit.char_name = "测试角色".to_owned();
        hit.attack_type = Some("普攻".to_owned());
        hit.damage_name = Some("普攻一段".to_owned());
        state.push_hit(hit);

        let summary = state
            .session_summary(
                CaptureQualitySource::JsonReplay,
                DpsTimeBasis::SubtractTimeStop,
                false,
            )
            .expect("summary should exist");

        assert_eq!(summary.dps_time_mode, DpsTimeBasis::SubtractTimeStop);
        assert_eq!(summary.total_damage, 120.0);
        assert_eq!(summary.total_hits, 1);
        assert_eq!(summary.characters[0].name, "测试角色");
        assert_eq!(summary.skills[0].name, "普攻一段");
        assert_eq!(summary.quality.source, CaptureQualitySource::JsonReplay);
    }

    #[test]
    fn combat_session_summary_uses_incremental_non_incoming_hit_count() {
        let mut state = CombatState::default();
        state.push_hit(test_hit(1.0, 10, "outgoing", 10.0));
        state.push_hit(test_hit(2.0, 10, "unknown", 0.0));
        state.push_hit(test_hit(3.0, 10, "incoming", 5.0));

        let summary = state
            .session_summary(CaptureQualitySource::Live, DpsTimeBasis::WallClock, false)
            .expect("mixed-direction summary");

        assert_eq!(summary.total_hits, 2);
        assert_eq!(state.skill_breakdown_index.directions.outgoing_hits, 1);
        assert_eq!(state.skill_breakdown_index.directions.unknown_hits, 1);
        assert_eq!(state.skill_breakdown_index.directions.incoming_hits, 1);
    }

    #[test]
    fn enemy_direction_backfill_updates_attribution_without_rebuilding_totals() {
        let mut state = CombatState::default();
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 0.0,
            cycle: Some(1),
            floor: Some(1),
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });
        let mut candidate = test_hit(1.0, 7, "unknown", 100.0);
        candidate.attack_type = Some("Normal Attack".to_owned());
        state.push_hit(candidate);
        reset_combat_total_rebuild_count();

        let timestamp_100ns =
            FILETIME_UNIX_EPOCH_100NS.saturating_add((1.05 * FILETIME_TICKS_PER_SECOND) as u64);
        let event = ModScriptEvent::from_bridge(
            1,
            timestamp_100ns,
            ENEMY_TELEMETRY_MOD_ID.to_owned(),
            "post.enemy.hit_target".to_owned(),
            vec![0x1234, 0x5678, 1],
        );
        assert_eq!(
            state.apply_mod_script_event(&event),
            ModScriptApplyOutcome::ProjectionChanged
        );

        assert_eq!(combat_total_rebuild_count(), 0);
        assert_eq!(state.total_damage, 100.0);
        assert_eq!(state.hits[0].direction, HitDirection::Outgoing);
        let global = state.stats.get(&7).expect("global character aggregate");
        assert_eq!(
            (global.attributed_hits, global.attributed_damage),
            (1, 100.0)
        );
        assert_eq!((global.direct_hits, global.direct_damage), (1, 100.0));
        let first = state
            .abyss
            .first_half
            .stats
            .get(&7)
            .expect("first-half character aggregate");
        assert_eq!((first.attributed_hits, first.attributed_damage), (1, 100.0));
        assert_eq!((first.direct_hits, first.direct_damage), (1, 100.0));
    }

    #[test]
    fn combat_session_summary_keeps_abyss_halves_separate() {
        let mut state = CombatState::default();
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 0.0,
            cycle: None,
            floor: Some(1),
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });
        let mut first = test_hit(1.0, 1, "outgoing", 100.0);
        first.char_name = "上半角色".to_owned();
        first.damage_name = Some("上半技能".to_owned());
        state.push_hit(first);
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 5.0,
            cycle: None,
            floor: Some(1),
            half: AbyssHalf::Second,
            allow_late_backfill: false,
        });
        let mut second = test_hit(6.0, 2, "outgoing", 200.0);
        second.char_name = "下半角色".to_owned();
        second.damage_name = Some("下半技能".to_owned());
        state.push_hit(second);

        let summary = state
            .session_summary(
                CaptureQualitySource::JsonReplay,
                DpsTimeBasis::SubtractTimeStop,
                false,
            )
            .expect("summary should exist");
        let first_half = summary.abyss.first_half.expect("first half summary");
        let second_half = summary.abyss.second_half.expect("second half summary");

        assert_eq!(summary.abyss.active_half, Some(AbyssHalf::Second));
        assert_eq!(first_half.half, AbyssHalf::First);
        assert_eq!(first_half.characters[0].name, "上半角色");
        assert_eq!(first_half.skills[0].name, "上半技能");
        assert_eq!(second_half.half, AbyssHalf::Second);
        assert_eq!(second_half.characters[0].name, "下半角色");
        assert_eq!(second_half.skills[0].name, "下半技能");
    }

    #[test]
    fn follow_up_damage_merges_into_source_hit_totals() {
        let mut state = CombatState::default();
        let mut hit = test_hit(1.0, 7, "outgoing", 1_000.0);
        hit.target_hp_before = 10_000.0;
        hit.target_hp_after = 9_000.0;
        hit.target_max_hp = 10_000.0;
        hit.gameplay_effect_index = Some(42);
        state.push_hit(hit);

        state.apply_follow_up(HitFollowUp {
            source_timestamp: 1.0,
            source_char_id: 7,
            source_damage: 1_000.0,
            source_target_hp_before: 10_000.0,
            source_target_hp_after: 9_000.0,
            source_target_max_hp: 10_000.0,
            source_gameplay_effect_index: Some(42),
            timestamp: 1.2,
            damage: 250.0,
            target_hp_after: 8_750.0,
            target_hp_percent: 87.5,
            damage_name: Some("覆纹追加攻击".to_owned()),
            attack_type: Some("覆纹".to_owned()),
            damage_attribute: Some("灵".to_owned()),
        });

        let merged = state.hits.front().unwrap();
        assert_eq!(merged.damage, 1_000.0);
        assert_eq!(merged.follow_up_damage, 250.0);
        assert_eq!(merged.target_hp_after, 8_750.0);
        assert_eq!(state.total_damage, 1_250.0);
        let stats = state.stats.get(&7).unwrap();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.damage, 1_250.0);
        assert_eq!(state.damage_correction_count, 0);
    }

    #[test]
    fn damage_correction_replaces_source_hit_totals() {
        let mut state = CombatState::default();
        let mut hit = test_hit(1.0, 7, "outgoing", 1_000.0);
        hit.target_hp_before = 10_000.0;
        hit.target_hp_after = 9_000.0;
        hit.target_max_hp = 10_000.0;
        hit.gameplay_effect_index = Some(42);
        state.push_hit(hit);

        state.apply_damage_correction(HitDamageCorrection {
            source_timestamp: 1.0,
            source_char_id: 7,
            source_damage: 1_000.0,
            source_target_hp_before: 10_000.0,
            source_target_hp_after: 9_000.0,
            source_target_max_hp: 10_000.0,
            source_gameplay_effect_index: Some(42),
            damage: 1_250.0,
            target_hp_before: 10_250.0,
            target_hp_after: 9_000.0,
            target_hp_percent: 90.0,
            max_hp_reduction: None,
            reconciled_overkill_damage: None,
        });

        let corrected = state.hits.front().unwrap();
        assert_eq!(corrected.damage, 1_250.0);
        assert_eq!(corrected.follow_up_damage, 0.0);
        assert_eq!(corrected.target_hp_before, 10_250.0);
        assert_eq!(state.total_damage, 1_250.0);
        let stats = state.stats.get(&7).unwrap();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.damage, 1_250.0);
        assert_eq!(state.damage_correction_count, 1);
    }

    #[test]
    fn recent_corrections_and_follow_ups_update_only_deltas_and_the_recorded_half() {
        let mut state = CombatState::default();
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 0.0,
            cycle: Some(1),
            floor: Some(1),
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });
        for index in 0..2_000 {
            state.push_hit(test_hit(index as f64 + 1.0, 1, "outgoing", 10.0));
        }
        let mut source = test_hit(3_000.0, 7, "outgoing", 100.0);
        source.byte_offset = 77;
        source.bit_shift = 3;
        source.target_hp_before = 1_000.0;
        source.target_hp_after = 900.0;
        source.target_max_hp = 1_000.0;
        source.gameplay_effect_index = Some(42);
        state.push_hit(source);

        assert_eq!(
            state.recent_hit_records.len(),
            RECENT_HIT_MUTATION_WINDOW,
            "the mutation index must stay independently bounded"
        );
        let global_generation = state.hits_generation;
        let first_generation = state.abyss.first_half.hits_generation;
        let second_generation = state.abyss.second_half.hits_generation;
        let total_before = state.total_damage;
        reset_combat_total_rebuild_count();

        assert!(state.apply_damage_correction(HitDamageCorrection {
            source_timestamp: 3_000.0,
            source_char_id: 7,
            source_damage: 100.0,
            source_target_hp_before: 1_000.0,
            source_target_hp_after: 900.0,
            source_target_max_hp: 1_000.0,
            source_gameplay_effect_index: Some(42),
            damage: 150.0,
            target_hp_before: 1_050.0,
            target_hp_after: 900.0,
            target_hp_percent: 90.0,
            max_hp_reduction: None,
            reconciled_overkill_damage: None,
        }));
        // The follow-up still names the original hit. The bounded record keeps
        // that source alias even though the correction changed damage/HP.
        assert!(state.apply_follow_up(HitFollowUp {
            source_timestamp: 3_000.0,
            source_char_id: 7,
            source_damage: 100.0,
            source_target_hp_before: 1_000.0,
            source_target_hp_after: 900.0,
            source_target_max_hp: 1_000.0,
            source_gameplay_effect_index: Some(42),
            timestamp: 3_000.1,
            damage: 25.0,
            target_hp_after: 875.0,
            target_hp_percent: 87.5,
            damage_name: Some("覆纹追加攻击".to_owned()),
            attack_type: Some("覆纹".to_owned()),
            damage_attribute: Some("灵".to_owned()),
        }));

        assert_eq!(combat_total_rebuild_count(), 0);
        assert_eq!(state.hits_generation, global_generation.wrapping_add(2));
        assert_eq!(
            state.abyss.first_half.hits_generation,
            first_generation.wrapping_add(2)
        );
        assert_eq!(
            state.abyss.second_half.hits_generation, second_generation,
            "the unrelated half must not be searched or mutated"
        );
        assert!((state.total_damage - (total_before + 75.0)).abs() < 1e-9);
        assert!((state.stats[&7].damage - 175.0).abs() < 1e-9);
        assert!((state.abyss.first_half.stats[&7].damage - 175.0).abs() < 1e-9);
        assert_eq!(state.damage_correction_count, 1);
    }

    #[test]
    fn damage_correction_requires_hp_identity_even_when_gameplay_effect_index_matches() {
        // Two hits from the same AoE application (identical char_id, timestamp,
        // and gameplay_effect_index — that index identifies the ability
        // application, not a single target) landing on two different targets.
        let mut state = CombatState::default();
        let mut first = test_hit(1.0, 7, "outgoing", 1_000.0);
        first.target_hp_before = 10_000.0;
        first.target_hp_after = 9_000.0;
        first.target_max_hp = 10_000.0;
        first.gameplay_effect_index = Some(42);
        state.push_hit(first);

        let mut second = test_hit(1.0, 7, "outgoing", 2_000.0);
        second.target_hp_before = 50_000.0;
        second.target_hp_after = 48_000.0;
        second.target_max_hp = 50_000.0;
        second.gameplay_effect_index = Some(42);
        state.push_hit(second);

        // A correction keyed off the FIRST hit's own HP fields must land on
        // that hit specifically, not on the second (more recently pushed, so
        // checked first by the reverse search) one sharing the same index.
        state.apply_damage_correction(HitDamageCorrection {
            source_timestamp: 1.0,
            source_char_id: 7,
            source_damage: 1_000.0,
            source_target_hp_before: 10_000.0,
            source_target_hp_after: 9_000.0,
            source_target_max_hp: 10_000.0,
            source_gameplay_effect_index: Some(42),
            damage: 1_250.0,
            target_hp_before: 10_000.0,
            target_hp_after: 8_750.0,
            target_hp_percent: 87.5,
            max_hp_reduction: None,
            reconciled_overkill_damage: None,
        });

        let first_stored = state
            .hits
            .iter()
            .find(|hit| hit.target_max_hp == 10_000.0)
            .unwrap();
        let second_stored = state
            .hits
            .iter()
            .find(|hit| hit.target_max_hp == 50_000.0)
            .unwrap();
        assert_eq!(first_stored.damage, 1_250.0, "targeted hit gets corrected");
        assert_eq!(
            second_stored.damage, 2_000.0,
            "untouched hit must not change"
        );
    }

    #[test]
    fn abyss_half_labels_are_utf8_chinese() {
        assert_eq!(AbyssHalf::First.label(), "Ascending Line");
        assert_eq!(AbyssHalf::Second.label(), "Descending Line");
    }

    #[test]
    fn battle_reset_preserves_inventory_and_its_generation() {
        let mut state = CombatState::default();
        state.push_hit(test_hit(1.0, 7, "outgoing", 100.0));
        state.replace_empty_curtain(vec![EmptyCurtainItem {
            id: HtItemNetId { solt: 1, serial: 2 },
            item_id: "item".to_owned(),
            level: 1,
            main_stats: Vec::new(),
            sub_stats: Vec::new(),
            locked: false,
            discarded: false,
            character_net_id: None,
            equipped_character_id: None,
            equipped_placement: None,
        }]);
        state.replace_empty_curtain_characters(vec![EmptyCurtainCharacter {
            net_id: HtItemNetId { solt: 3, serial: 4 },
            character_id: 1020,
        }]);
        let inventory_generation = state.empty_curtain_generation;
        let character_generation = state.empty_curtain_characters_generation;

        state.clear_battle_preserving_inventory();

        assert!(state.hits.is_empty());
        assert!(state.stats.is_empty());
        assert_eq!(state.total_damage, 0.0);
        assert_eq!(state.empty_curtain.len(), 1);
        assert_eq!(state.empty_curtain_characters.len(), 1);
        assert_eq!(state.empty_curtain_generation, inventory_generation);
        assert_eq!(
            state.empty_curtain_characters_generation,
            character_generation
        );
    }

    #[test]
    fn party_combat_retains_every_hit_and_lifetime_total() {
        let mut state = PartyCombatState::default();
        let hits = long_combat_hits();
        let expected_generation = hits.len() as u64;
        let expected_hit_count = hits.len();
        for hit in hits {
            state.push_hit(hit);
        }

        assert_eq!(state.hits_generation, expected_generation);
        assert_eq!(state.hits.len(), expected_hit_count);
        assert_totals_match_all_hits(
            &state.hits,
            &state.stats,
            state.total_damage,
            state.total_damage_taken,
            state.duration_with_time_stop(true),
        );
        assert_eq!(state.started_at, Some(-100_000.0));
        assert_eq!(
            state.stats.get(&99).map(|row| row.damage),
            Some(1_000_000.0)
        );
    }

    #[test]
    fn combat_retains_every_hit_and_lifetime_total() {
        let mut state = CombatState::default();
        let hits = long_combat_hits();
        let expected_generation = hits.len() as u64;
        let expected_hit_count = hits.len();
        for hit in hits {
            state.push_hit(hit);
        }

        assert_eq!(state.hits_generation, expected_generation);
        assert_eq!(state.hits.len(), expected_hit_count);
        assert_totals_match_all_hits(
            &state.hits,
            &state.stats,
            state.total_damage,
            state.total_damage_taken,
            state.duration_with_time_stop(true),
        );
        assert_eq!(state.started_at, Some(-100_000.0));
        assert_eq!(
            state.stats.get(&99).map(|row| row.damage),
            Some(1_000_000.0)
        );
    }

    #[test]
    fn unbalance_damage_counts_toward_team_total_but_not_personal_ranking() {
        let mut state = PartyCombatState::default();
        state.push_hit(test_hit(10.0, 1021, "outgoing", 100.0));

        let mut unbalance_hit = test_hit(11.0, 1021, "outgoing", 5_000.0);
        unbalance_hit.attack_type = Some("倾陷伤害".to_owned());
        state.push_hit(unbalance_hit);

        assert_eq!(state.total_damage, 5_100.0);
        let stats = state.stats.get(&1021).unwrap();
        assert_eq!(stats.damage, 100.0);
        assert_eq!(stats.hits, 1);
    }

    #[test]
    fn reaction_damage_types_only_include_confirmed_follow_up_damage() {
        for attack_type in REACTION_DAMAGE_TYPES {
            assert!(is_reaction_damage_type(attack_type));
        }
        for attack_type in ["环合·创生", "普攻", UNBALANCE_ATTACK_TYPE] {
            assert!(!is_reaction_damage_type(attack_type));
        }
    }

    #[test]
    fn damage_attribution_closes_team_total_and_projects_character_policy() {
        let mut state = CombatState::default();

        let mut direct = test_hit(1.0, 1, "outgoing", 100.0);
        direct.attack_type = Some("普攻".to_owned());
        direct.follow_up_damage = 20.0;
        direct.follow_up_attack_type = Some("创生花".to_owned());
        state.push_hit(direct);

        let mut reaction = test_hit(2.0, 1, "outgoing", 25.0);
        reaction.attack_type = Some("覆纹".to_owned());
        state.push_hit(reaction);

        let mut shared = test_hit(3.0, 1, "outgoing", 30.0);
        shared.attack_type = Some(UNBALANCE_ATTACK_TYPE.to_owned());
        state.push_hit(shared);

        let mut unknown_character = test_hit(4.0, 900_001, "outgoing", 40.0);
        unknown_character.char_known = false;
        state.push_hit(unknown_character);

        state.push_hit(test_hit(5.0, 1, "unknown", 50.0));

        let attribution = state.damage_attribution_summary();
        assert_eq!(attribution.total_damage, 265.0);
        assert_eq!(attribution.character_direct_damage, 100.0);
        assert_eq!(attribution.character_reaction_damage, 45.0);
        assert_eq!(attribution.shared_damage, 30.0);
        assert_eq!(attribution.unattributed_damage, 90.0);
        assert_eq!(
            attribution.character_damage(false)
                + attribution.shared_damage
                + attribution.unattributed_damage,
            attribution.total_damage
        );
        assert_eq!(
            attribution.character_damage(true)
                + attribution.character_reaction_damage
                + attribution.shared_damage
                + attribution.unattributed_damage,
            attribution.total_damage
        );

        let row = state.stats.get(&1).expect("known character row");
        let included = row.for_reaction_damage_policy(false);
        let separated = row.for_reaction_damage_policy(true);
        assert_eq!((included.hits, included.damage), (2, 145.0));
        assert_eq!((separated.hits, separated.damage), (1, 100.0));
        assert_eq!((included.first_hit, included.last_hit), (1.0, 2.0));
        assert_eq!((separated.first_hit, separated.last_hit), (1.0, 1.0));
    }

    #[test]
    fn session_summary_records_reaction_damage_policy_without_changing_team_total() {
        let mut state = CombatState::default();
        let mut direct = test_hit(1.0, 1, "outgoing", 100.0);
        direct.attack_type = Some("普攻".to_owned());
        state.push_hit(direct);
        let mut reaction = test_hit(2.0, 1, "outgoing", 25.0);
        reaction.attack_type = Some("创生花".to_owned());
        state.push_hit(reaction);

        let included = state
            .session_summary(
                CaptureQualitySource::JsonReplay,
                DpsTimeBasis::WallClock,
                false,
            )
            .expect("included summary");
        let separated = state
            .session_summary(
                CaptureQualitySource::JsonReplay,
                DpsTimeBasis::WallClock,
                true,
            )
            .expect("separated summary");

        assert_eq!(included.total_damage, separated.total_damage);
        assert_eq!(included.total_damage, 125.0);
        assert_eq!(included.characters[0].damage, 125.0);
        assert_eq!(separated.characters[0].damage, 100.0);
        assert!(!included.reaction_damage_separated);
        assert!(separated.reaction_damage_separated);
        assert_eq!(included.damage_attribution, separated.damage_attribution);
    }

    #[test]
    fn combat_duration_uses_observed_game_pause_boundaries() {
        let mut state = CombatState::default();
        state.push_hit(test_hit(10.0, 1021, "outgoing", 100.0));
        state.apply_time_stop_event(TimeStopEvent::GamePauseStarted {
            timestamp: 12.25,
            pause_type_mask: 1 << 2,
        });
        assert!(state.is_game_paused());
        state.apply_time_stop_event(TimeStopEvent::GamePauseEnded {
            timestamp: 15.75,
            pause_type_mask: 1 << 2,
        });
        assert!(!state.is_game_paused());
        state.push_hit(test_hit(20.0, 1021, "outgoing", 200.0));

        assert!((state.duration_with_time_stop(false) - 10.0).abs() < 1e-9);
        assert!((state.duration_with_time_stop(true) - 6.5).abs() < 1e-9);
        assert!((state.active_elapsed_between(10.0, 20.0) - 6.5).abs() < 1e-9);
        assert!((state.dps_with_time_stop(true) - (300.0 / 6.5)).abs() < 1e-9);
    }

    #[test]
    fn time_stop_authority_and_export_events_compact_to_bounded_state() {
        let pause_count = MAX_RETAINED_TIME_STOP_INTERVALS + 1_000;
        let mut state = CombatState::default();
        state.push_hit(test_hit(0.0, 1, "outgoing", 100.0));
        for index in 0..pause_count {
            let start = index as f64 * 3.0 + 1.0;
            apply_test_pause(&mut state, start, start + 1.0);
        }
        let end = pause_count as f64 * 3.0 + 1.0;
        state.push_hit(test_hit(end, 1, "outgoing", 100.0));

        assert!(state.time_stop.intervals.len() <= MAX_RETAINED_TIME_STOP_INTERVALS);
        assert!(state.time_stop.archived.is_some());
        assert!(state.time_stop_events.len() <= MAX_RETAINED_TIME_STOP_EVENTS);
        let projected = state.time_stop_intervals_between(0.0, end);
        assert!(projected.len() <= MAX_PROJECTED_TIME_STOP_INTERVALS);
        let timeline = state.timeline_bounded(1.0, true, 32, 8, 8);
        assert!(timeline.time_stop_intervals.len() <= MAX_PROJECTED_TIME_STOP_INTERVALS);
        assert!(timeline.compacted_time_stop_intervals > 0);
        assert!((state.time_stop.frozen_between(0.0, end) - pause_count as f64).abs() < 1e-9);

        // The bounded event projection remains self-contained for JSON/history
        // reconstruction: compacted gaps may move, but full-span frozen time is
        // exact and does not silently disappear.
        let mut restored = CombatState::default();
        restored.push_hit(test_hit(0.0, 1, "outgoing", 100.0));
        for event in state.time_stop_events.clone() {
            restored.apply_time_stop_event(event);
        }
        restored.push_hit(test_hit(end, 1, "outgoing", 100.0));
        assert!(
            (restored.duration_with_time_stop(true) - state.duration_with_time_stop(true)).abs()
                < 1e-9
        );
    }

    #[test]
    fn timeline_marker_source_cap_is_enforced_before_string_projection() {
        let mut markers = Vec::new();
        for index in 0..(MAX_TIMELINE_MARKERS + 10) {
            push_timeline_marker(
                &mut markers,
                Some(index as f64),
                0.0,
                1_000.0,
                "marker",
                TimelineMarkerKind::HalfStart,
            );
        }
        assert_eq!(markers.len(), MAX_TIMELINE_MARKERS);
    }

    #[test]
    fn observed_game_pause_end_advances_the_combat_clock_without_inflating_active_time() {
        let mut state = CombatState::default();
        state.push_hit(test_hit(10.0, 1021, "outgoing", 100.0));
        state.push_hit(test_hit(20.0, 1021, "outgoing", 200.0));
        state.apply_time_stop_event(TimeStopEvent::GamePauseStarted {
            timestamp: 20.0,
            pause_type_mask: 1 << 3,
        });
        state.apply_time_stop_event(TimeStopEvent::GamePauseEnded {
            timestamp: 23.0,
            pause_type_mask: 1 << 3,
        });

        assert!((state.duration_with_time_stop(false) - 13.0).abs() < 1e-9);
        assert!((state.duration_with_time_stop(true) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn combat_duration_subtracts_authoritative_pause_interval() {
        let mut state = CombatState::default();
        state.push_hit(test_hit(10.0, 1021, "outgoing", 100.0));
        apply_test_pause(&mut state, 11.0, 14.0);
        state.push_hit(test_hit(20.0, 1021, "outgoing", 200.0));

        assert!((state.duration_with_time_stop(true) - 7.0).abs() < 1e-9);
        assert!((state.dps_with_time_stop(true) - (300.0 / 7.0)).abs() < 1e-9);
    }

    #[test]
    fn combat_duration_only_subtracts_the_part_after_the_first_hit() {
        let mut state = CombatState::default();
        apply_test_pause(&mut state, 10.0, 14.0);
        state.push_hit(test_hit(13.0, 1021, "outgoing", 100.0));
        state.push_hit(test_hit(20.0, 1021, "outgoing", 200.0));

        assert!((state.duration_with_time_stop(false) - 7.0).abs() < 1e-9);
        assert!((state.duration_with_time_stop(true) - 6.0).abs() < 1e-9);

        let mut completed_before_combat = CombatState::default();
        apply_test_pause(&mut completed_before_combat, 5.0, 9.0);
        completed_before_combat.push_hit(test_hit(10.0, 1021, "outgoing", 100.0));
        completed_before_combat.push_hit(test_hit(20.0, 1021, "outgoing", 200.0));

        assert!((completed_before_combat.duration_with_time_stop(true) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn early_support_pause_without_damage_does_not_deduct_precombat_time() {
        let mut state = CombatState::default();
        state.apply_time_stop_event(TimeStopEvent::GamePauseStarted {
            timestamp: 10.0,
            pause_type_mask: 1 << 2,
        });
        state.apply_time_stop_event(TimeStopEvent::GamePauseEnded {
            timestamp: 14.0,
            pause_type_mask: 1 << 2,
        });
        state.push_hit(test_hit(13.0, 1021, "outgoing", 100.0));
        state.push_hit(test_hit(20.0, 1021, "outgoing", 200.0));

        assert!((state.duration_with_time_stop(false) - 7.0).abs() < 1e-9);
        assert!((state.duration_with_time_stop(true) - 6.0).abs() < 1e-9);
    }

    #[test]
    fn pause_start_and_end_edges_control_the_clock_immediately() {
        let mut state = CombatState::default();
        state.push_hit(test_hit(10.0, 1021, "outgoing", 100.0));
        state.apply_time_stop_event(TimeStopEvent::GamePauseStarted {
            timestamp: 12.0,
            pause_type_mask: 1 << 2,
        });
        assert!((state.duration_with_time_stop(false) - 2.0).abs() < 1e-9);
        assert!((state.duration_with_time_stop(true) - 2.0).abs() < 1e-9);

        state.apply_time_stop_event(TimeStopEvent::GamePauseEnded {
            timestamp: 16.0,
            pause_type_mask: 1 << 2,
        });
        state.push_hit(test_hit(20.0, 1021, "outgoing", 200.0));

        assert!((state.duration_with_time_stop(false) - 10.0).abs() < 1e-9);
        assert!((state.duration_with_time_stop(true) - 6.0).abs() < 1e-9);
    }

    #[test]
    fn abyss_duration_uses_authoritative_pause_state_edges() {
        let mut state = CombatState::default();
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 10.0,
            cycle: Some(6),
            floor: Some(12),
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });
        state.push_hit(test_hit(13.7, 1021, "outgoing", 100.0));
        state.apply_time_stop_event(TimeStopEvent::GamePauseStarted {
            timestamp: 15.0,
            pause_type_mask: 1 << 3,
        });
        state.apply_time_stop_event(TimeStopEvent::GamePauseEnded {
            timestamp: 20.0,
            pause_type_mask: 1 << 3,
        });
        state.push_hit(test_hit(66.1, 1021, "outgoing", 200.0));

        assert!((state.abyss.first_half.duration_with_time_stop(false) - 52.4).abs() < 1e-9);
        assert!((state.abyss.first_half.duration_with_time_stop(true) - 47.4).abs() < 1e-9);
    }

    #[test]
    fn settlement_stage_never_becomes_the_duration() {
        let mut state = CombatState::default();
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 10.0,
            cycle: Some(6),
            floor: Some(12),
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });
        state.push_hit(test_hit(13.7, 1021, "outgoing", 100.0));
        state.push_hit(test_hit(63.0, 1021, "outgoing", 200.0));
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 66.183,
            cycle: None,
            floor: None,
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });

        assert!((state.abyss.first_half.duration_with_time_stop(false) - 49.3).abs() < 1e-9);
        assert!((state.abyss.first_half.duration_with_time_stop(true) - 49.3).abs() < 1e-9);
    }

    #[test]
    fn session_summary_time_basis_controls_duration_and_dps() {
        let mut state = CombatState::default();
        state.push_hit(test_hit(10.0, 1021, "outgoing", 100.0));
        apply_test_pause(&mut state, 11.0, 14.0);
        state.push_hit(test_hit(20.0, 1021, "outgoing", 200.0));

        let adjusted = state
            .session_summary(
                CaptureQualitySource::Unknown,
                DpsTimeBasis::SubtractTimeStop,
                false,
            )
            .expect("adjusted summary should exist");
        let wall_clock = state
            .session_summary(
                CaptureQualitySource::Unknown,
                DpsTimeBasis::WallClock,
                false,
            )
            .expect("wall-clock summary should exist");

        assert_eq!(adjusted.dps_time_mode, DpsTimeBasis::SubtractTimeStop);
        assert!((adjusted.duration_seconds - 7.0).abs() < 1e-9);
        assert!((adjusted.total_dps - (300.0 / 7.0)).abs() < 1e-9);
        assert_eq!(wall_clock.dps_time_mode, DpsTimeBasis::WallClock);
        assert!((wall_clock.duration_seconds - 10.0).abs() < 1e-9);
        assert!((wall_clock.total_dps - 30.0).abs() < 1e-9);
    }

    #[test]
    fn character_duration_subtracts_time_stop() {
        let mut state = CombatState::default();
        state.push_hit(test_hit(10.0, 1021, "outgoing", 100.0));
        apply_test_pause(&mut state, 11.0, 14.0);
        state.push_hit(test_hit(20.0, 1021, "outgoing", 200.0));

        let row = state.stats.get(&1021).unwrap();
        assert!((row.duration() - 10.0).abs() < 1e-9);
        assert!((state.character_duration_with_time_stop(row, true) - 7.0).abs() < 1e-9);
        assert!((state.character_dps_with_time_stop(row, true) - (300.0 / 7.0)).abs() < 1e-9);
        assert!((state.duration_with_time_stop(false) - 10.0).abs() < 1e-9);
        assert!((state.dps_with_time_stop(false) - 30.0).abs() < 1e-9);
        assert!((state.character_duration_with_time_stop(row, false) - 10.0).abs() < 1e-9);
        assert!((state.character_dps_with_time_stop(row, false) - 30.0).abs() < 1e-9);
    }

    #[test]
    fn abyss_active_half_duration_subtracts_time_stop() {
        let mut state = CombatState::default();
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 0.0,
            cycle: Some(1),
            floor: Some(1),
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });
        state.push_hit(test_hit(1.0, 1010, "outgoing", 100.0));
        apply_test_pause(&mut state, 2.0, 4.0);
        state.push_hit(test_hit(6.0, 1010, "outgoing", 100.0));

        assert_eq!(state.abyss.first_half.started_at, Some(1.0));
        assert!((state.abyss.first_half.duration_with_time_stop(true) - 3.0).abs() < 1e-9);
        let row = state.abyss.first_half.stats.get(&1010).unwrap();
        assert!(
            (state
                .abyss
                .first_half
                .character_duration_with_time_stop(row, true)
                - 3.0)
                .abs()
                < 1e-9
        );
    }

    #[test]
    fn authoritative_stage_assigns_half_without_starting_damage_clock() {
        let mut state = CombatState::default();
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 1.0,
            cycle: None,
            floor: None,
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 3.0,
            cycle: Some(5),
            floor: Some(12),
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });
        state.push_hit(test_hit(4.0, 1010, "outgoing", 100.0));
        let mut last_hit = test_hit(10.0, 1010, "outgoing", 100.0);
        last_hit.target_hp_before = 1_000.0;
        last_hit.target_hp_after = 900.0;
        last_hit.target_max_hp = 1_000.0;
        last_hit.gameplay_effect_index = Some(42);
        state.push_hit(last_hit);

        state.apply_damage_correction(HitDamageCorrection {
            source_timestamp: 10.0,
            source_char_id: 1010,
            source_damage: 100.0,
            source_target_hp_before: 1_000.0,
            source_target_hp_after: 900.0,
            source_target_max_hp: 1_000.0,
            source_gameplay_effect_index: Some(42),
            damage: 110.0,
            target_hp_before: 1_010.0,
            target_hp_after: 900.0,
            target_hp_percent: 90.0,
            max_hp_reduction: None,
            reconciled_overkill_damage: None,
        });

        assert_eq!(state.abyss.first_half.started_at, Some(4.0));
        assert!((state.abyss.first_half.duration_with_time_stop(true) - 6.0).abs() < 1e-9);
        assert!(
            state
                .global_hit_axis_page(0, usize::MAX)
                .all(|(_, half)| half == Some(AbyssHalf::First)),
            "damage correction must not detach axis half membership"
        );
    }

    #[test]
    fn abyss_half_clips_pause_to_the_damage_window() {
        let mut state = CombatState::default();
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 1.0,
            cycle: Some(5),
            floor: Some(12),
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });
        apply_test_pause(&mut state, 2.0, 6.0);
        state.push_hit(test_hit(5.0, 1010, "outgoing", 100.0));
        state.push_hit(test_hit(10.0, 1010, "outgoing", 200.0));

        assert_eq!(state.abyss.first_half.started_at, Some(5.0));
        assert!((state.abyss.first_half.duration_with_time_stop(false) - 5.0).abs() < 1e-9);
        assert!((state.abyss.first_half.duration_with_time_stop(true) - 4.0).abs() < 1e-9);
    }

    #[test]
    fn late_detected_second_half_backfills_existing_global_hits() {
        let mut state = CombatState::default();
        state.push_hit(test_hit(10.0, 1010, "outgoing", 100.0));
        apply_test_pause(&mut state, 11.0, 13.0);
        state.push_hit(test_hit(15.0, 1010, "outgoing", 200.0));

        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 20.0,
            cycle: None,
            floor: None,
            half: AbyssHalf::Second,
            allow_late_backfill: true,
        });
        state.apply_abyss_event(AbyssEvent::Success { timestamp: 21.0 });

        assert_eq!(state.total_damage, 300.0);
        assert_eq!(state.abyss.first_half.total_damage, 0.0);
        assert_eq!(state.abyss.second_half.total_damage, 300.0);
        assert_eq!(state.abyss.second_half.hits.len(), 2);
        assert!((state.abyss.second_half.duration_with_time_stop(true) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn late_backfilled_half_accepts_delayed_pause_before_stage_timestamp() {
        let mut state = CombatState::default();
        state.push_hit(test_hit(10.0, 1010, "outgoing", 100.0));
        state.push_hit(test_hit(15.0, 1010, "outgoing", 200.0));

        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 20.0,
            cycle: None,
            floor: None,
            half: AbyssHalf::Second,
            allow_late_backfill: true,
        });
        apply_test_pause(&mut state, 11.0, 13.0);

        assert!((state.duration_with_time_stop(true) - 3.0).abs() < 1e-9);
        assert!((state.abyss.second_half.duration_with_time_stop(true) - 3.0).abs() < 1e-9);
        assert_eq!(state.abyss.second_half.time_stop.intervals.len(), 1);
    }

    #[test]
    fn delayed_hit_after_half_switch_keeps_original_character_half() {
        let mut state = CombatState::default();
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 1.0,
            cycle: Some(5),
            floor: Some(12),
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });
        state.push_hit(test_hit(2.0, 1076, "outgoing", 100.0));

        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 3.0,
            cycle: Some(5),
            floor: Some(12),
            half: AbyssHalf::Second,
            allow_late_backfill: false,
        });
        state.push_hit(test_hit(3.5, 1076, "outgoing", 20.0));
        state.push_hit(test_hit(3.6, 1052, "outgoing", 200.0));

        assert_eq!(state.abyss.first_half.hits.len(), 2);
        assert_eq!(state.abyss.first_half.total_damage, 120.0);
        assert_eq!(state.abyss.second_half.hits.len(), 1);
        assert_eq!(state.abyss.second_half.total_damage, 200.0);
        assert!(state.abyss.second_half.stats.contains_key(&1052));
        assert!(!state.abyss.second_half.stats.contains_key(&1076));
    }

    #[test]
    fn restart_releases_cleared_character_half() {
        let mut state = CombatState::default();
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 1.0,
            cycle: None,
            floor: Some(12),
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });
        state.push_hit(test_hit(2.0, 1076, "outgoing", 100.0));
        state.apply_abyss_event(AbyssEvent::RestartDetected { timestamp: 3.0 });
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 4.0,
            cycle: None,
            floor: Some(12),
            half: AbyssHalf::Second,
            allow_late_backfill: false,
        });
        state.push_hit(test_hit(5.0, 1076, "outgoing", 200.0));

        assert!(state.abyss.first_half.hits.is_empty());
        assert_eq!(state.abyss.second_half.hits.len(), 1);
        assert_eq!(state.abyss.second_half.total_damage, 200.0);
    }

    #[test]
    fn restart_from_second_to_first_clears_both_halves() {
        let mut state = CombatState::default();
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 1.0,
            cycle: Some(6),
            floor: Some(12),
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });
        state.push_hit(test_hit(2.0, 1076, "outgoing", 100.0));
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 3.0,
            cycle: Some(6),
            floor: Some(12),
            half: AbyssHalf::Second,
            allow_late_backfill: false,
        });
        state.push_hit(test_hit(4.0, 1052, "outgoing", 200.0));

        state.apply_abyss_event(AbyssEvent::RestartDetected { timestamp: 5.0 });
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 5.0,
            cycle: None,
            floor: None,
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });

        assert_eq!(state.abyss.active_half, Some(AbyssHalf::First));
        assert!(state.abyss.first_half.hits.is_empty());
        assert!(state.abyss.second_half.hits.is_empty());
        assert_eq!(state.abyss.first_half.total_damage, 0.0);
        assert_eq!(state.abyss.second_half.total_damage, 0.0);

        state.push_hit(test_hit(6.0, 1076, "outgoing", 300.0));
        assert_eq!(state.abyss.first_half.total_damage, 300.0);
        assert_eq!(state.abyss.second_half.total_damage, 0.0);
    }

    #[test]
    fn next_floor_start_clears_previous_floor_after_long_transition() {
        let mut state = CombatState::default();
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 1.0,
            cycle: Some(6),
            floor: Some(11),
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });
        state.push_hit(test_hit(2.0, 1076, "outgoing", 100.0));
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 3.0,
            cycle: Some(6),
            floor: Some(11),
            half: AbyssHalf::Second,
            allow_late_backfill: false,
        });
        state.push_hit(test_hit(4.0, 1052, "outgoing", 200.0));
        state.apply_abyss_event(AbyssEvent::Success { timestamp: 5.0 });
        state.apply_abyss_event(AbyssEvent::RestartDetected { timestamp: 6.0 });

        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 25.0,
            cycle: Some(6),
            floor: Some(12),
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });

        assert_eq!(state.abyss.floor, Some(12));
        assert_eq!(state.abyss.active_half, Some(AbyssHalf::First));
        assert!(state.abyss.first_half.hits.is_empty());
        assert!(state.abyss.second_half.hits.is_empty());
        assert_eq!(state.abyss.first_half.total_damage, 0.0);
        assert_eq!(state.abyss.second_half.total_damage, 0.0);
        assert_eq!(state.abyss.first_half_at, Some(25.0));
        assert_eq!(state.abyss.success_at, None);
        assert_eq!(state.abyss.pending_restart_at, None);
        assert_eq!(state.abyss.pending_restart_half, None);
    }

    #[test]
    fn unknown_character_hits_follow_the_active_half() {
        let mut state = CombatState::default();
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 1.0,
            cycle: None,
            floor: Some(12),
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });
        let mut first = test_hit(2.0, 0, "outgoing", 100.0);
        first.char_known = false;
        state.push_hit(first);

        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 3.0,
            cycle: None,
            floor: Some(12),
            half: AbyssHalf::Second,
            allow_late_backfill: false,
        });
        let mut second = test_hit(4.0, 0, "outgoing", 200.0);
        second.char_known = false;
        state.push_hit(second);

        assert_eq!(state.abyss.first_half.total_damage, 100.0);
        assert_eq!(state.abyss.second_half.total_damage, 200.0);
    }

    #[test]
    fn first_normal_stage_does_not_backfill_previous_global_hits() {
        let mut state = CombatState::default();
        state.push_hit(test_hit(1.0, 1010, "outgoing", 100.0));

        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 20.0,
            cycle: None,
            floor: None,
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });

        assert_eq!(state.total_damage, 100.0);
        assert_eq!(state.abyss.first_half.total_damage, 0.0);
        assert!(state.abyss.first_half.hits.is_empty());
    }

    fn timeline_from_pattern(pattern: &[bool]) -> TimelineSeries {
        let buckets = pattern
            .iter()
            .enumerate()
            .map(|(index, &active)| TimelineBucket {
                start_offset: index as f64,
                end_offset: (index + 1) as f64,
                damage: if active { 100.0 } else { 0.0 },
                hits: u64::from(active),
                ..Default::default()
            })
            .collect();
        TimelineSeries {
            bucket_seconds: 1.0,
            buckets,
            ..Default::default()
        }
    }

    #[test]
    fn combat_segments_split_on_long_idle_gap() {
        // 3 active buckets, a 6s idle gap (> 5s), then 2 active buckets.
        let series = timeline_from_pattern(&[
            true, true, true, false, false, false, false, false, false, true, true,
        ]);
        let segments = summarize_combat_segments(&series, 5.0);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].start_offset, 0.0);
        assert_eq!(segments[0].end_offset, 3.0);
        assert_eq!(segments[0].total_damage, 300.0);
        assert_eq!(segments[0].hits, 3);
        assert_eq!(segments[0].duration, 3.0);
        assert_eq!(segments[0].dps, 100.0);
        assert_eq!(segments[1].start_offset, 9.0);
        assert_eq!(segments[1].total_damage, 200.0);
    }

    #[test]
    fn combat_segments_keep_short_gaps_together() {
        // A 2s gap (< 5s) does not split the fight.
        let series = timeline_from_pattern(&[true, false, false, true]);
        let segments = summarize_combat_segments(&series, 5.0);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].total_damage, 200.0);
        assert_eq!(segments[0].hits, 2);
    }

    #[test]
    fn combat_segments_empty_series_has_no_segments() {
        assert!(summarize_combat_segments(&TimelineSeries::default(), 5.0).is_empty());
    }
}
