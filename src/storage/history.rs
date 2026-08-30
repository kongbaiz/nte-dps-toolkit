use std::fmt;
use std::fs;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{CloseHandle, FILETIME, GetLastError},
    System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
};

use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize, de::IgnoredAny};

use crate::engine::model::{
    AbyssHalf, CaptureQualitySummary, CombatSessionAbyssHalfSummary, CombatSessionCharacterSummary,
    CombatSessionSkillSummary, CombatSessionSummary, CombatState, DamageAttributionSummary, Hit,
    TeamDps, TeamDpsMember, TimeStopEvent,
};
use crate::storage::io_util::{atomic_write_file, atomic_write_text};
use crate::storage::paths::software_dir;

pub const HISTORY_RECORD_VERSION: u32 = 1;
pub const MAX_HISTORY_RECORDS: usize = 200;
const MAX_HISTORY_IMPORT_HITS: usize = 500_000;
const MAX_HISTORY_IMPORT_TIME_STOP_EVENTS: usize = 8_192;
pub const MAX_HISTORY_IMPORT_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_HISTORY_INTERACTIVE_DETAIL_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_HISTORY_INTERACTIVE_DETAIL_HITS: u64 = 1_000_000;
pub const MAX_HISTORY_SUMMARY_CHARACTERS_PER_SCOPE: usize = 256;
pub const MAX_HISTORY_SUMMARY_SKILLS_PER_SCOPE: usize = 4_096;
pub const MAX_HISTORY_SUMMARY_TEXT_BYTES: usize = 256;
const MAX_HISTORY_SUMMARY_SCOPES: usize = 3;
const HISTORY_DETAILS_CHUNK_VERSION: u32 = 1;
const MAX_HISTORY_HITS_PER_CHUNK: usize = 20_000;
const MAX_HISTORY_CHUNK_BYTES: u64 = 16 * 1024 * 1024;
const MAX_HISTORY_CHUNKS_PER_RECORD: usize = 4_096;
const MAX_HISTORY_TOMBSTONE_FILES_PER_RECORD: usize = MAX_HISTORY_CHUNKS_PER_RECORD + 2;
const MAX_HISTORY_DETAILS_BYTES: u64 = 4 * 1024 * 1024 * 1024;
pub const MAX_HISTORY_DIRECTORY_BYTES: u64 = MAX_HISTORY_DETAILS_BYTES;
/// Absolute root-directory enumeration budget. A fully retained History set may
/// legitimately contain one main manifest plus 4,096 detail sidecars per
/// record, so sidecars must not consume the independent main-manifest budget.
pub const MAX_HISTORY_DIRECTORY_ENTRIES: usize =
    MAX_HISTORY_RECORDS * (MAX_HISTORY_CHUNKS_PER_RECORD + 1) + MAX_HISTORY_RECORDS + 64;
pub const MAX_HISTORY_MAIN_MANIFEST_CANDIDATES: usize = 16_384;
const HISTORY_TOMBSTONE_OWNER_VERSION: u32 = 1;
const HISTORY_TOMBSTONE_OWNER_FILE: &str = ".owner.json";
const MAX_HISTORY_TOMBSTONE_OWNER_BYTES: u64 = 4 * 1024;
const MAX_HISTORY_TOMBSTONE_AGE_MILLIS: i64 = 24 * 60 * 60 * 1_000;

#[derive(Clone, Copy)]
struct HistoryDirectoryLimits {
    max_entries: usize,
    max_main_manifests: usize,
    max_records: usize,
    max_total_bytes: u64,
}

impl HistoryDirectoryLimits {
    const PRODUCTION: Self = Self {
        max_entries: MAX_HISTORY_DIRECTORY_ENTRIES,
        max_main_manifests: MAX_HISTORY_MAIN_MANIFEST_CANDIDATES,
        max_records: MAX_HISTORY_RECORDS,
        max_total_bytes: MAX_HISTORY_DIRECTORY_BYTES,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistoryFileReadError {
    NotFile,
    Metadata,
    TooLarge,
    Read,
    Utf8,
}

impl fmt::Display for HistoryFileReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotFile => "History record path is not a file",
            Self::Metadata => "History record metadata could not be read",
            Self::TooLarge => "History record exceeds the supported file size",
            Self::Read => "History record could not be read",
            Self::Utf8 => "History record must contain UTF-8 JSON",
        })
    }
}

static HISTORY_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryRecord {
    pub version: u32,
    pub id: String,
    pub saved_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<DateTime<Utc>>,
    pub summary: CombatSessionSummary,
    pub details: Option<HistoryCombatDetails>,
    /// Internal on-disk storage manifest. Public/exported records are hydrated
    /// and clear this field, so external JSON remains a self-contained legacy
    /// document subject to the normal import budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[doc(hidden)]
    pub details_chunks: Option<HistoryDetailsChunkManifest>,
}

impl Default for HistoryRecord {
    fn default() -> Self {
        Self {
            version: HISTORY_RECORD_VERSION,
            id: String::new(),
            saved_at: Utc::now(),
            recorded_at: None,
            summary: CombatSessionSummary::default(),
            details: None,
            details_chunks: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[doc(hidden)]
pub enum HistoryHitLane {
    Global,
    FirstHalf,
    SecondHalf,
}

impl HistoryHitLane {
    const fn file_token(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::FirstHalf => "first",
            Self::SecondHalf => "second",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[doc(hidden)]
pub struct HistoryHitChunkRef {
    lane: HistoryHitLane,
    index: usize,
    hit_count: usize,
    bytes: u64,
    checksum: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[doc(hidden)]
pub struct HistoryDetailsChunkManifest {
    version: u32,
    metadata: HistoryCombatDetails,
    chunks: Vec<HistoryHitChunkRef>,
    total_hits: u64,
    total_bytes: u64,
}

#[derive(Serialize)]
struct HistoryHitChunkWrite<'a> {
    version: u32,
    record_id: &'a str,
    lane: HistoryHitLane,
    index: usize,
    hits: &'a [Hit],
}

#[derive(Serialize)]
struct HistoryRecordDisk<'a> {
    version: u32,
    id: &'a str,
    saved_at: &'a DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recorded_at: Option<&'a DateTime<Utc>>,
    summary: &'a CombatSessionSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<&'a HistoryCombatDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    details_chunks: Option<&'a HistoryDetailsChunkManifest>,
}

#[derive(Deserialize)]
struct HistoryHitChunkRead {
    version: u32,
    record_id: String,
    lane: HistoryHitLane,
    index: usize,
    hits: Vec<Hit>,
}

#[derive(Deserialize)]
#[serde(default)]
struct HistoryRecordExportEnvelope {
    version: u32,
    id: String,
    saved_at: DateTime<Utc>,
    recorded_at: Option<DateTime<Utc>>,
    summary: CombatSessionSummary,
    details: Option<IgnoredAny>,
    details_chunks: Option<HistoryDetailsChunkManifest>,
}

impl Default for HistoryRecordExportEnvelope {
    fn default() -> Self {
        Self {
            version: HISTORY_RECORD_VERSION,
            id: String::new(),
            saved_at: DateTime::<Utc>::UNIX_EPOCH,
            recorded_at: None,
            summary: CombatSessionSummary::default(),
            details: None,
            details_chunks: None,
        }
    }
}

enum PreparedHistoryRecordExportStorage {
    /// Legacy inline records and summary-only records are already self-contained.
    /// Their source identity is rebound while the atomic destination is written.
    SelfContained,
    Chunked(Box<HistoryDetailsChunkManifest>),
}

/// A bounded, hit-free descriptor prepared without hydrating detail chunks.
/// The potentially multi-gigabyte chunk stream is opened only by the later
/// export step, which revalidates the main manifest identity.
pub struct PreparedHistoryRecordExport {
    main_path: PathBuf,
    main_bytes: u64,
    main_checksum: u64,
    version: u32,
    id: String,
    saved_at: DateTime<Utc>,
    recorded_at: Option<DateTime<Utc>>,
    summary: CombatSessionSummary,
    storage: PreparedHistoryRecordExportStorage,
}

struct HistoryChecksumReader<R> {
    inner: R,
    bytes_read: u64,
    checksum: u64,
}

impl<R> HistoryChecksumReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            bytes_read: 0,
            checksum: 0xcbf2_9ce4_8422_2325,
        }
    }
}

impl<R: Read> Read for HistoryChecksumReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.bytes_read = self.bytes_read.saturating_add(read as u64);
        self.checksum = history_chunk_checksum_update(self.checksum, &buffer[..read]);
        Ok(read)
    }
}

impl HistoryRecord {
    pub fn effective_timestamp(&self) -> &DateTime<Utc> {
        self.recorded_at.as_ref().unwrap_or(&self.saved_at)
    }

    pub fn display_time(&self) -> String {
        self.effective_timestamp()
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    }

    pub fn file_timestamp(&self) -> String {
        self.effective_timestamp()
            .with_timezone(&Local)
            .format("%Y%m%d_%H%M%S")
            .to_string()
    }

    pub fn to_team_dps(&self) -> Option<TeamDps> {
        team_from_characters(self.summary.total_dps, &self.summary.characters)
    }

    pub fn upper_team_dps(&self) -> Option<TeamDps> {
        self.summary
            .abyss
            .first_half
            .as_ref()
            .and_then(|half| team_from_characters(half.total_dps, &half.characters))
            .or_else(|| self.to_team_dps())
    }

    pub fn lower_team_dps(&self) -> Option<TeamDps> {
        self.summary
            .abyss
            .second_half
            .as_ref()
            .and_then(|half| team_from_characters(half.total_dps, &half.characters))
            .or_else(|| self.to_team_dps())
    }
}

/// A failure before the destination record crosses the atomic-write commit
/// boundary. Retrying an archive after one of these failures cannot duplicate
/// a committed record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistorySaveError {
    InvalidSummary,
    InvalidDetails,
    PrepareDirectory,
    Serialize,
    TooLarge,
    Commit,
}

impl fmt::Display for HistorySaveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSummary => "History summary failed validation.",
            Self::InvalidDetails => "History details failed validation.",
            Self::PrepareDirectory => "History storage directory could not be prepared.",
            Self::Serialize => "History record could not be serialized.",
            Self::TooLarge => "History record exceeds the supported file size.",
            Self::Commit => "History record could not be committed.",
        })
    }
}

impl std::error::Error for HistorySaveError {}

impl HistorySaveError {
    /// Only filesystem preparation/commit failures may succeed unchanged on a
    /// later attempt. Validation, serialization, and a single record that
    /// exceeds the chunked storage envelope are permanent for that archive and
    /// must not be placed in an infinite retry loop.
    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::PrepareDirectory | Self::Commit)
    }
}

/// A post-commit retention task that did not finish. The newly written record
/// remains committed and must not be submitted to the record retry queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryMaintenanceWarning {
    RetentionPruneFailed,
}

impl fmt::Display for HistoryMaintenanceWarning {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RetentionPruneFailed => {
                formatter.write_str("History retention maintenance did not finish.")
            }
        }
    }
}

/// The result after a History record has crossed the atomic-write commit
/// boundary. Both variants contain the single committed record identity.
#[derive(Clone, Debug)]
#[must_use = "a committed History record may also carry a maintenance warning"]
pub enum HistorySaveOutcome {
    Committed(HistoryRecord),
    CommittedWithMaintenanceWarning {
        record: HistoryRecord,
        warning: HistoryMaintenanceWarning,
    },
}

/// Lightweight commit result for callers that retain ownership of an
/// unbounded archive and therefore must not receive a second `HistoryRecord`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorrowedHistorySaveOutcome {
    Committed,
    CommittedWithMaintenanceWarning(HistoryMaintenanceWarning),
}

impl HistorySaveOutcome {
    pub fn record(&self) -> &HistoryRecord {
        match self {
            Self::Committed(record) | Self::CommittedWithMaintenanceWarning { record, .. } => {
                record
            }
        }
    }

    pub fn maintenance_warning(&self) -> Option<HistoryMaintenanceWarning> {
        match self {
            Self::Committed(_) => None,
            Self::CommittedWithMaintenanceWarning { warning, .. } => Some(*warning),
        }
    }

    pub fn into_record(self) -> HistoryRecord {
        match self {
            Self::Committed(record) | Self::CommittedWithMaintenanceWarning { record, .. } => {
                record
            }
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryCombatDetails {
    pub floor: Option<u32>,
    pub active_half: Option<AbyssHalf>,
    pub first_half_at: Option<f64>,
    pub second_half_at: Option<f64>,
    pub success_at: Option<f64>,
    pub exited_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub global_hits: Vec<Hit>,
    pub first_half_hits: Vec<Hit>,
    pub second_half_hits: Vec<Hit>,
    pub time_stop_events: Vec<TimeStopEvent>,
    /// Provider health is frozen with the round. Older records deserialize as
    /// Unknown and therefore never pretend that time-stop adjustment was
    /// authoritative.
    pub combat_clock_health: crate::engine::model::CombatClockRuntimeHealth,
}

impl HistoryCombatDetails {
    fn metadata_only(&self) -> Self {
        Self {
            floor: self.floor,
            active_half: self.active_half,
            first_half_at: self.first_half_at,
            second_half_at: self.second_half_at,
            success_at: self.success_at,
            exited_at: self.exited_at,
            global_hits: Vec::new(),
            first_half_hits: Vec::new(),
            second_half_hits: Vec::new(),
            time_stop_events: self.time_stop_events.clone(),
            combat_clock_health: self.combat_clock_health,
        }
    }

    pub fn from_state(state: &CombatState) -> Option<Self> {
        let abyss = &state.abyss;
        let has_abyss_hits =
            !abyss.first_half.hits.is_empty() || !abyss.second_half.hits.is_empty();
        if !has_abyss_hits && state.hits.is_empty() {
            return None;
        }
        let (round_started_at, round_ended_at) = if has_abyss_hits {
            (
                [
                    abyss.first_half_at,
                    abyss.second_half_at,
                    abyss.first_half.started_at,
                    abyss.second_half.started_at,
                ]
                .into_iter()
                .flatten()
                .min_by(f64::total_cmp),
                [
                    abyss.first_half.ended_at,
                    abyss.second_half.ended_at,
                    abyss.success_at,
                    abyss.exited_at,
                ]
                .into_iter()
                .flatten()
                .max_by(f64::total_cmp),
            )
        } else {
            (state.started_at, state.ended_at)
        };
        let round_started_at = round_started_at?;
        let round_ended_at = round_ended_at?;
        Some(Self {
            floor: if has_abyss_hits { abyss.floor } else { None },
            active_half: if has_abyss_hits {
                abyss.active_half
            } else {
                None
            },
            first_half_at: if has_abyss_hits {
                abyss.first_half_at
            } else {
                None
            },
            second_half_at: if has_abyss_hits {
                abyss.second_half_at
            } else {
                None
            },
            success_at: if has_abyss_hits {
                abyss.success_at
            } else {
                None
            },
            exited_at: if has_abyss_hits {
                abyss.exited_at
            } else {
                None
            },
            global_hits: if has_abyss_hits {
                Vec::new()
            } else {
                state.hits.iter().cloned().collect()
            },
            first_half_hits: abyss.first_half.hits.iter().cloned().collect(),
            second_half_hits: abyss.second_half.hits.iter().cloned().collect(),
            time_stop_events: clipped_time_stop_events(
                &state.time_stop_events,
                round_started_at,
                round_ended_at,
            ),
            combat_clock_health: archived_combat_clock_health(state.combat_clock_health),
        })
    }

    /// Consumes a detached round and moves every unbounded hit payload into
    /// History storage. This is the archive hot path; unlike [`Self::from_state`]
    /// it never clones hit strings or target context vectors.
    pub fn from_state_owned(mut state: CombatState) -> Option<Self> {
        let abyss = &state.abyss;
        let has_abyss_hits =
            !abyss.first_half.hits.is_empty() || !abyss.second_half.hits.is_empty();
        if !has_abyss_hits && state.hits.is_empty() {
            return None;
        }
        let (round_started_at, round_ended_at) = if has_abyss_hits {
            (
                [
                    abyss.first_half_at,
                    abyss.second_half_at,
                    abyss.first_half.started_at,
                    abyss.second_half.started_at,
                ]
                .into_iter()
                .flatten()
                .min_by(f64::total_cmp),
                [
                    abyss.first_half.ended_at,
                    abyss.second_half.ended_at,
                    abyss.success_at,
                    abyss.exited_at,
                ]
                .into_iter()
                .flatten()
                .max_by(f64::total_cmp),
            )
        } else {
            (state.started_at, state.ended_at)
        };
        let round_started_at = round_started_at?;
        let round_ended_at = round_ended_at?;
        let time_stop_events = clipped_time_stop_events_owned(
            std::mem::take(&mut state.time_stop_events),
            round_started_at,
            round_ended_at,
        );
        Some(Self {
            floor: has_abyss_hits.then_some(state.abyss.floor).flatten(),
            active_half: has_abyss_hits.then_some(state.abyss.active_half).flatten(),
            first_half_at: has_abyss_hits
                .then_some(state.abyss.first_half_at)
                .flatten(),
            second_half_at: has_abyss_hits
                .then_some(state.abyss.second_half_at)
                .flatten(),
            success_at: has_abyss_hits.then_some(state.abyss.success_at).flatten(),
            exited_at: has_abyss_hits.then_some(state.abyss.exited_at).flatten(),
            global_hits: if has_abyss_hits {
                Vec::new()
            } else {
                std::mem::take(&mut state.hits).into_iter().collect()
            },
            first_half_hits: std::mem::take(&mut state.abyss.first_half.hits)
                .into_iter()
                .collect(),
            second_half_hits: std::mem::take(&mut state.abyss.second_half.hits)
                .into_iter()
                .collect(),
            time_stop_events,
            combat_clock_health: archived_combat_clock_health(state.combat_clock_health),
        })
    }

    pub fn to_combat_state(&self) -> CombatState {
        let mut state = CombatState::default();
        state.combat_clock_health = archived_combat_clock_health(self.combat_clock_health);
        state.abyss.floor = self.floor;
        state.abyss.active_half = self.active_half;
        state.abyss.first_half_at = self.first_half_at;
        state.abyss.second_half_at = self.second_half_at;
        state.abyss.success_at = self.success_at;
        state.abyss.exited_at = self.exited_at;
        state.replace_global_hits_bulk(self.global_hits.clone());
        state
            .abyss
            .first_half
            .replace_hits_bulk(self.first_half_hits.clone());
        state
            .abyss
            .second_half
            .replace_hits_bulk(self.second_half_hits.clone());
        for event in &self.time_stop_events {
            state.apply_time_stop_event(event.clone());
        }
        if self.global_hits.is_empty() {
            state.rebuild_global_from_abyss();
        }
        state
    }

    /// Rebuilds a selected History state by moving its unbounded vectors.
    pub fn into_combat_state(self) -> CombatState {
        let Self {
            floor,
            active_half,
            first_half_at,
            second_half_at,
            success_at,
            exited_at,
            global_hits,
            first_half_hits,
            second_half_hits,
            time_stop_events,
            combat_clock_health,
        } = self;
        let mut state = CombatState::default();
        state.combat_clock_health = archived_combat_clock_health(combat_clock_health);
        state.abyss.floor = floor;
        state.abyss.active_half = active_half;
        state.abyss.first_half_at = first_half_at;
        state.abyss.second_half_at = second_half_at;
        state.abyss.success_at = success_at;
        state.abyss.exited_at = exited_at;
        state.replace_global_hits_bulk(global_hits);
        state.abyss.first_half.replace_hits_bulk(first_half_hits);
        state.abyss.second_half.replace_hits_bulk(second_half_hits);
        for event in time_stop_events {
            state.apply_time_stop_event(event);
        }
        if state.hits.is_empty() {
            state.rebuild_global_from_abyss();
        }
        state
    }

    fn recorded_at(&self) -> Option<DateTime<Utc>> {
        self.global_hits
            .iter()
            .chain(&self.first_half_hits)
            .chain(&self.second_half_hits)
            .map(|hit| hit.timestamp)
            .min_by(f64::total_cmp)
            .and_then(unix_seconds_to_utc)
    }

    fn validate(&self) -> Result<(), String> {
        if !self.global_hits.is_empty()
            && (!self.first_half_hits.is_empty() || !self.second_half_hits.is_empty())
        {
            return Err("History detail mixes global and abyss hits".to_owned());
        }
        for timestamp in [
            self.first_half_at,
            self.second_half_at,
            self.success_at,
            self.exited_at,
        ]
        .into_iter()
        .flatten()
        {
            if !timestamp.is_finite() {
                return Err("History detail contains an invalid timestamp".to_owned());
            }
        }
        if !history_hits_are_valid(&self.global_hits)
            || !history_hits_are_valid(&self.first_half_hits)
            || !history_hits_are_valid(&self.second_half_hits)
        {
            return Err("History detail contains an invalid hit".to_owned());
        }
        if self.time_stop_events.iter().any(|event| {
            let timestamp = match event {
                TimeStopEvent::GamePauseStarted { timestamp, .. }
                | TimeStopEvent::GamePauseEnded { timestamp, .. } => timestamp,
            };
            !timestamp.is_finite()
        }) {
            return Err("History detail contains an invalid time-stop event".to_owned());
        }
        if self.time_stop_events.len() > MAX_HISTORY_IMPORT_TIME_STOP_EVENTS {
            return Err("History time-stop event count exceeds the storage limit".to_owned());
        }
        Ok(())
    }

    fn validate_external(&self) -> Result<(), String> {
        self.validate()?;
        if self
            .global_hits
            .len()
            .saturating_add(self.first_half_hits.len())
            .saturating_add(self.second_half_hits.len())
            > MAX_HISTORY_IMPORT_HITS
        {
            return Err("History detail hit count exceeds the import limit".to_owned());
        }
        if self.time_stop_events.len() > MAX_HISTORY_IMPORT_TIME_STOP_EVENTS {
            return Err("History time-stop event count exceeds the import limit".to_owned());
        }
        Ok(())
    }
}

fn history_detail_hit_count(details: &HistoryCombatDetails) -> u64 {
    (details.global_hits.len() as u64)
        .saturating_add(details.first_half_hits.len() as u64)
        .saturating_add(details.second_half_hits.len() as u64)
}

fn history_hits_are_valid(hits: &[Hit]) -> bool {
    !hits.iter().any(|hit| {
        !hit.timestamp.is_finite()
            || !hit.damage.is_finite()
            || !hit.follow_up_damage.is_finite()
            || !hit.max_hp_reduction.is_finite()
            || hit.max_hp_reduction < 0.0
            || hit
                .follow_up_timestamp
                .is_some_and(|timestamp| !timestamp.is_finite())
    })
}

const fn archived_combat_clock_health(
    health: crate::engine::model::CombatClockRuntimeHealth,
) -> crate::engine::model::CombatClockRuntimeHealth {
    match health {
        crate::engine::model::CombatClockRuntimeHealth::Available => {
            crate::engine::model::CombatClockRuntimeHealth::Recorded
        }
        health => health,
    }
}

fn clipped_time_stop_events(
    events: &[TimeStopEvent],
    range_start: f64,
    range_end: f64,
) -> Vec<TimeStopEvent> {
    let mut clipped = Vec::new();
    let mut active_pause: Option<(f64, u32)> = None;
    for event in events {
        match event {
            TimeStopEvent::GamePauseStarted {
                timestamp,
                pause_type_mask,
            } => match &mut active_pause {
                Some((start, active_mask)) => {
                    *start = start.min(*timestamp);
                    *active_mask |= *pause_type_mask;
                }
                None => active_pause = Some((*timestamp, *pause_type_mask)),
            },
            TimeStopEvent::GamePauseEnded {
                timestamp,
                pause_type_mask,
            } => {
                let Some((start, active_mask)) = active_pause.take() else {
                    continue;
                };
                let start = start.max(range_start);
                let end = timestamp.min(range_end);
                if end <= start {
                    continue;
                }
                clipped.push(TimeStopEvent::GamePauseStarted {
                    timestamp: start,
                    pause_type_mask: active_mask,
                });
                clipped.push(TimeStopEvent::GamePauseEnded {
                    timestamp: end,
                    pause_type_mask: *pause_type_mask,
                });
            }
        }
    }
    clipped
}

fn clipped_time_stop_events_owned(
    events: Vec<TimeStopEvent>,
    range_start: f64,
    range_end: f64,
) -> Vec<TimeStopEvent> {
    let mut clipped = Vec::new();
    let mut active_pause: Option<(f64, u32)> = None;
    for event in events {
        match event {
            TimeStopEvent::GamePauseStarted {
                timestamp,
                pause_type_mask,
            } => match &mut active_pause {
                Some((start, active_mask)) => {
                    *start = start.min(timestamp);
                    *active_mask |= pause_type_mask;
                }
                None => active_pause = Some((timestamp, pause_type_mask)),
            },
            TimeStopEvent::GamePauseEnded {
                timestamp,
                pause_type_mask,
            } => {
                let Some((start, active_mask)) = active_pause.take() else {
                    continue;
                };
                let start = start.max(range_start);
                let end = timestamp.min(range_end);
                if end <= start {
                    continue;
                }
                clipped.push(TimeStopEvent::GamePauseStarted {
                    timestamp: start,
                    pause_type_mask: active_mask,
                });
                clipped.push(TimeStopEvent::GamePauseEnded {
                    timestamp: end,
                    pause_type_mask,
                });
            }
        }
    }
    clipped
}

fn unix_seconds_to_utc(timestamp: f64) -> Option<DateTime<Utc>> {
    let timestamp_millis = timestamp * 1_000.0;
    if !timestamp_millis.is_finite()
        || timestamp_millis < i64::MIN as f64
        || timestamp_millis > i64::MAX as f64
    {
        return None;
    }
    DateTime::<Utc>::from_timestamp_millis(timestamp_millis.round() as i64)
}

fn team_from_characters(dps: f64, characters: &[CombatSessionCharacterSummary]) -> Option<TeamDps> {
    (dps > 0.0).then(|| TeamDps {
        dps,
        members: characters
            .iter()
            .filter(|row| row.damage > 0.0)
            .take(crate::engine::model::TEAM_DPS_MAX_MEMBERS)
            .map(|row| TeamDpsMember {
                id: row.char_id,
                dps: row.dps,
                name: row.name.clone(),
            })
            .collect(),
    })
}

#[derive(Clone, Debug, Default)]
pub struct HistoryLoadResult {
    pub records: Vec<HistoryRecord>,
    pub skipped_files: usize,
}

#[derive(Clone, Debug, Default)]
pub struct HistoryIndexRecord {
    pub path: PathBuf,
    pub id: String,
    pub display_time: String,
    pub abyss_floor: Option<u32>,
    pub has_details: bool,
    /// Exact aggregate sidecar bytes for chunked details, or the conservative
    /// main-file byte size for a legacy inline record.
    pub stored_detail_bytes: u64,
    pub total_detail_hits: Option<u64>,
    effective_timestamp: DateTime<Utc>,
    detail_chunks: Vec<HistoryHitChunkRef>,
}

#[derive(Clone, Debug, Default)]
pub struct HistoryIndexLoadResult {
    pub records: Vec<HistoryIndexRecord>,
    pub skipped_files: usize,
}

#[derive(Deserialize)]
#[serde(default)]
struct HistoryIndexEnvelope {
    version: u32,
    id: String,
    saved_at: DateTime<Utc>,
    recorded_at: Option<DateTime<Utc>>,
    summary: HistoryIndexSummary,
    details: Option<IgnoredAny>,
    details_chunks: Option<HistoryDetailsChunkManifestIndex>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct HistoryDetailsChunkManifestIndex {
    version: u32,
    metadata: Option<IgnoredAny>,
    chunks: Vec<HistoryHitChunkRef>,
    total_hits: u64,
    total_bytes: u64,
}

#[derive(Deserialize)]
#[serde(default)]
struct HistorySummaryEnvelope {
    version: u32,
    id: String,
    saved_at: DateTime<Utc>,
    recorded_at: Option<DateTime<Utc>>,
    summary: CombatSessionSummary,
    details: Option<IgnoredAny>,
    details_chunks: Option<HistoryDetailsChunkManifestIndex>,
}

impl Default for HistorySummaryEnvelope {
    fn default() -> Self {
        Self {
            version: HISTORY_RECORD_VERSION,
            id: String::new(),
            saved_at: DateTime::<Utc>::UNIX_EPOCH,
            recorded_at: None,
            summary: CombatSessionSummary::default(),
            details: None,
            details_chunks: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryRecordLoadError {
    InvalidId,
    DetailsTooLarge { size: u64, limit: u64 },
    DetailHitsTooLarge { count: u64, limit: u64 },
    LoadFailed,
}

impl fmt::Display for HistoryRecordLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId => formatter.write_str("Invalid history record ID"),
            Self::DetailsTooLarge { .. } => {
                formatter.write_str("History record details exceed the requested load budget")
            }
            Self::DetailHitsTooLarge { .. } => {
                formatter.write_str("History record hit count exceeds the requested load budget")
            }
            Self::LoadFailed => formatter.write_str("History record could not be loaded"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryRecordExportError {
    InvalidId,
    NotFound,
    SourceChanged,
    CorruptRecord,
    DestinationWriteFailed,
}

impl fmt::Display for HistoryRecordExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidId => "Invalid history record ID",
            Self::NotFound => "History record was not found",
            Self::SourceChanged => "History record changed while it was being exported",
            Self::CorruptRecord => "History record failed its export integrity check",
            Self::DestinationWriteFailed => "History export destination could not be written",
        })
    }
}

impl std::error::Error for HistoryRecordLoadError {}

#[derive(Clone, Debug)]
pub struct HistoryDeleteTombstone {
    token: String,
    record_id: String,
    directory: PathBuf,
    owner: HistoryTombstoneFile,
    main: HistoryTombstoneFile,
    chunks: Vec<HistoryTombstoneFile>,
}

impl HistoryDeleteTombstone {
    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn record_id(&self) -> &str {
        &self.record_id
    }
}

#[derive(Clone, Debug)]
struct HistoryTombstoneFile {
    original: PathBuf,
    tombstone: PathBuf,
    expected_bytes: u64,
    checksum: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryTombstoneOwner {
    version: u32,
    token: String,
    owner_pid: u32,
    owner_started_at_ticks: Option<u64>,
    created_at_millis: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TombstoneOwnerStatus {
    Active,
    Dead,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryDeleteError {
    InvalidId,
    ScanFailed,
    ScanBudgetExceeded,
    CorruptDetails,
    PrepareFailed,
    MoveFailed,
    RollbackFailed,
    RestoreConflict,
    CleanupFailed,
}

impl fmt::Display for HistoryDeleteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidId => "Invalid history record ID",
            Self::ScanFailed => "History directory could not be scanned",
            Self::ScanBudgetExceeded => "History directory exceeds the supported scan budget",
            Self::CorruptDetails => "History record details failed integrity validation",
            Self::PrepareFailed => "History undo tombstone could not be prepared",
            Self::MoveFailed => "History record could not be moved into undo storage",
            Self::RollbackFailed => "History undo transaction rollback did not finish",
            Self::RestoreConflict => "History undo destination is no longer available",
            Self::CleanupFailed => "History undo tombstone could not be cleaned up",
        })
    }
}

impl std::error::Error for HistoryDeleteError {}

impl Default for HistoryIndexEnvelope {
    fn default() -> Self {
        Self {
            version: HISTORY_RECORD_VERSION,
            id: String::new(),
            saved_at: DateTime::<Utc>::UNIX_EPOCH,
            recorded_at: None,
            summary: HistoryIndexSummary::default(),
            details: None,
            details_chunks: None,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct HistoryIndexSummary {
    abyss: HistoryIndexAbyssSummary,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct HistoryIndexAbyssSummary {
    floor: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistoryComparison {
    pub left_id: String,
    pub right_id: String,
    pub total_dps_delta: f64,
    pub total_damage_delta: f64,
    pub duration_delta: f64,
    pub character_deltas: Vec<HistoryCharacterDelta>,
    pub skill_deltas: Vec<HistorySkillDelta>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistoryCharacterDelta {
    pub char_id: u32,
    pub name: String,
    pub left_dps: f64,
    pub right_dps: f64,
    pub delta_dps: f64,
    pub left_damage: f64,
    pub right_damage: f64,
    pub delta_damage: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistorySkillDelta {
    pub name: String,
    pub category: String,
    pub ability_name: Option<String>,
    pub gameplay_effect_name: Option<String>,
    pub left_damage: f64,
    pub right_damage: f64,
    pub delta_damage: f64,
}

pub fn history_dir() -> PathBuf {
    software_dir().join("history")
}

/// Loads bounded History row metadata without opening or materializing detail
/// chunks. Use this for list/stream projections; hydrate one selected record
/// with [`load_history_record_by_id_for_interactive_selection`] only when its
/// details are requested by the interactive presentation.
pub fn load_history_summaries() -> HistoryLoadResult {
    load_history_summaries_from_dir(&history_dir())
}

pub fn load_history_record_from_path(path: &Path) -> Result<HistoryRecord, String> {
    let (text, _) = read_history_text_with_limit(path, MAX_HISTORY_IMPORT_BYTES)
        .map_err(|error| error.to_string())?;
    parse_stored_history_record(&text, path, MAX_HISTORY_DETAILS_BYTES).map(|(record, _)| record)
}

/// Resolves and hydrates exactly one retained record instead of loading every
/// record's potentially large detail stream.
/// Preflights the trusted on-disk detail footprint from the lightweight index
/// before any chunk is opened or any `Hit` is materialized.
pub fn load_history_record_by_id_with_max_detail_bytes(
    record_id: &str,
    max_detail_bytes: u64,
) -> Result<Option<HistoryRecord>, HistoryRecordLoadError> {
    load_history_record_by_id_from_dir_with_max_detail_bytes(
        &history_dir(),
        record_id,
        max_detail_bytes,
    )
}

pub fn load_history_record_by_id_from_dir_with_max_detail_bytes(
    directory: &Path,
    record_id: &str,
    max_detail_bytes: u64,
) -> Result<Option<HistoryRecord>, HistoryRecordLoadError> {
    if !valid_record_id(record_id) {
        return Err(HistoryRecordLoadError::InvalidId);
    }
    let index = load_history_index_from_dir(directory);
    let Some(record) = index
        .records
        .into_iter()
        .find(|record| record.id == record_id)
    else {
        return Ok(None);
    };
    load_indexed_history_record_with_limits(&record, max_detail_bytes, None).map(Some)
}

/// Hydrates one user-selected round under a budget independent from external
/// import and full streaming export. Chunked byte and hit totals are rejected
/// from the lightweight main manifest before the first sidecar is opened.
pub fn load_history_record_by_id_for_interactive_selection(
    record_id: &str,
) -> Result<Option<HistoryRecord>, HistoryRecordLoadError> {
    load_history_record_by_id_from_dir_for_interactive_selection(&history_dir(), record_id)
}

pub fn load_history_record_by_id_from_dir_for_interactive_selection(
    directory: &Path,
    record_id: &str,
) -> Result<Option<HistoryRecord>, HistoryRecordLoadError> {
    if !valid_record_id(record_id) {
        return Err(HistoryRecordLoadError::InvalidId);
    }
    let index = load_history_index_from_dir(directory);
    let Some(record) = index
        .records
        .into_iter()
        .find(|record| record.id == record_id)
    else {
        return Ok(None);
    };
    load_indexed_history_record_with_limits(
        &record,
        MAX_HISTORY_INTERACTIVE_DETAIL_BYTES,
        Some(MAX_HISTORY_INTERACTIVE_DETAIL_HITS),
    )
    .map(Some)
}

fn load_indexed_history_record_with_limits(
    record: &HistoryIndexRecord,
    max_detail_bytes: u64,
    max_detail_hits: Option<u64>,
) -> Result<HistoryRecord, HistoryRecordLoadError> {
    if record.stored_detail_bytes > max_detail_bytes {
        return Err(HistoryRecordLoadError::DetailsTooLarge {
            size: record.stored_detail_bytes,
            limit: max_detail_bytes,
        });
    }
    if let (Some(count), Some(limit)) = (record.total_detail_hits, max_detail_hits)
        && count > limit
    {
        return Err(HistoryRecordLoadError::DetailHitsTooLarge { count, limit });
    }
    let loaded = load_history_record_from_path(&record.path)
        .map_err(|_| HistoryRecordLoadError::LoadFailed)?;
    if let (Some(details), Some(limit)) = (&loaded.details, max_detail_hits) {
        let count = history_detail_hit_count(details);
        if count > limit {
            return Err(HistoryRecordLoadError::DetailHitsTooLarge { count, limit });
        }
    }
    Ok(loaded)
}

/// Prepares a hit-free export descriptor from the local History index and main
/// manifest. The descriptor and later stream are identity-bound, so read-only
/// callers need not block History mutation/archive transactions.
pub fn prepare_history_record_export(
    record_id: &str,
) -> Result<PreparedHistoryRecordExport, HistoryRecordExportError> {
    prepare_history_record_export_from_dir(&history_dir(), record_id)
}

pub fn prepare_history_record_export_from_dir(
    directory: &Path,
    record_id: &str,
) -> Result<PreparedHistoryRecordExport, HistoryRecordExportError> {
    if !valid_record_id(record_id) {
        return Err(HistoryRecordExportError::InvalidId);
    }
    let index = load_history_index_from_dir(directory);
    let indexed = index
        .records
        .into_iter()
        .find(|record| record.id == record_id)
        .ok_or(HistoryRecordExportError::NotFound)?;
    let (text, main_bytes) = read_history_text_with_limit(&indexed.path, MAX_HISTORY_IMPORT_BYTES)
        .map_err(|_| HistoryRecordExportError::SourceChanged)?;
    let main_checksum = history_chunk_checksum(text.as_bytes());
    let mut envelope: HistoryRecordExportEnvelope =
        serde_json::from_str(&text).map_err(|_| HistoryRecordExportError::CorruptRecord)?;
    if envelope.version > HISTORY_RECORD_VERSION {
        return Err(HistoryRecordExportError::CorruptRecord);
    }
    if envelope.id.trim().is_empty() {
        envelope.id = indexed
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("legacy")
            .to_owned();
    }
    if envelope.id != record_id || !valid_record_id(&envelope.id) {
        return Err(HistoryRecordExportError::CorruptRecord);
    }
    validate_history_summary(&envelope.summary)
        .map_err(|_| HistoryRecordExportError::CorruptRecord)?;
    if envelope.details.is_some() && envelope.details_chunks.is_some() {
        return Err(HistoryRecordExportError::CorruptRecord);
    }
    let storage = match envelope.details_chunks {
        Some(manifest) => {
            if !history_chunk_manifest_is_valid(&manifest) || manifest.metadata.validate().is_err()
            {
                return Err(HistoryRecordExportError::CorruptRecord);
            }
            PreparedHistoryRecordExportStorage::Chunked(Box::new(manifest))
        }
        None => PreparedHistoryRecordExportStorage::SelfContained,
    };
    Ok(PreparedHistoryRecordExport {
        main_path: indexed.path,
        main_bytes,
        main_checksum,
        version: envelope.version,
        id: envelope.id,
        saved_at: envelope.saved_at,
        recorded_at: envelope.recorded_at,
        summary: envelope.summary,
        storage,
    })
}

/// Streams one retained record to a self-contained JSON document. Chunked hit
/// lanes are opened, bounded, integrity-checked, decoded, serialized, and
/// dropped one chunk at a time. The atomic destination is never committed when
/// the manifest or any sidecar moved or changed after descriptor preparation.
pub fn export_prepared_history_record_to_path(
    prepared: &PreparedHistoryRecordExport,
    destination: &Path,
) -> Result<(), HistoryRecordExportError> {
    let mut write_error = None;
    let result = atomic_write_file(destination, |writer| {
        let result = match &prepared.storage {
            PreparedHistoryRecordExportStorage::SelfContained => {
                copy_self_contained_history_export(prepared, writer)
            }
            PreparedHistoryRecordExportStorage::Chunked(manifest) => {
                verify_prepared_history_main(prepared)
                    .and_then(|()| write_chunked_history_export(prepared, manifest, writer))
            }
        };
        if let Err(error) = result {
            write_error = Some(error);
            return Err(error.to_string());
        }
        Ok(())
    });
    if let Some(error) = write_error {
        return Err(error);
    }
    result.map_err(|_| HistoryRecordExportError::DestinationWriteFailed)
}

pub fn export_history_record_by_id_from_dir_to_path(
    directory: &Path,
    record_id: &str,
    destination: &Path,
) -> Result<(), HistoryRecordExportError> {
    let prepared = prepare_history_record_export_from_dir(directory, record_id)?;
    export_prepared_history_record_to_path(&prepared, destination)
}

fn source_file_for_history_export(
    path: &Path,
    expected_bytes: u64,
) -> Result<fs::File, HistoryRecordExportError> {
    let file = fs::File::open(path).map_err(|_| HistoryRecordExportError::SourceChanged)?;
    let metadata = file
        .metadata()
        .map_err(|_| HistoryRecordExportError::SourceChanged)?;
    if !metadata.is_file() || metadata.len() != expected_bytes {
        return Err(HistoryRecordExportError::SourceChanged);
    }
    Ok(file)
}

fn copy_self_contained_history_export(
    prepared: &PreparedHistoryRecordExport,
    writer: &mut impl Write,
) -> Result<(), HistoryRecordExportError> {
    let mut file = source_file_for_history_export(&prepared.main_path, prepared.main_bytes)?;
    let mut remaining = prepared.main_bytes;
    let mut checksum = 0xcbf2_9ce4_8422_2325_u64;
    let mut buffer = [0_u8; 64 * 1024];
    while remaining > 0 {
        let requested = usize::try_from(remaining.min(buffer.len() as u64))
            .map_err(|_| HistoryRecordExportError::CorruptRecord)?;
        let read = file
            .read(&mut buffer[..requested])
            .map_err(|_| HistoryRecordExportError::SourceChanged)?;
        if read == 0 {
            return Err(HistoryRecordExportError::SourceChanged);
        }
        checksum = history_chunk_checksum_update(checksum, &buffer[..read]);
        writer
            .write_all(&buffer[..read])
            .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)?;
        remaining = remaining.saturating_sub(read as u64);
    }
    let mut trailing = [0_u8; 1];
    if file
        .read(&mut trailing)
        .map_err(|_| HistoryRecordExportError::SourceChanged)?
        != 0
        || checksum != prepared.main_checksum
    {
        return Err(HistoryRecordExportError::SourceChanged);
    }
    Ok(())
}

fn verify_prepared_history_main(
    prepared: &PreparedHistoryRecordExport,
) -> Result<(), HistoryRecordExportError> {
    let mut file = source_file_for_history_export(&prepared.main_path, prepared.main_bytes)?;
    let mut remaining = prepared.main_bytes;
    let mut checksum = 0xcbf2_9ce4_8422_2325_u64;
    let mut buffer = [0_u8; 64 * 1024];
    while remaining > 0 {
        let requested = usize::try_from(remaining.min(buffer.len() as u64))
            .map_err(|_| HistoryRecordExportError::CorruptRecord)?;
        let read = file
            .read(&mut buffer[..requested])
            .map_err(|_| HistoryRecordExportError::SourceChanged)?;
        if read == 0 {
            return Err(HistoryRecordExportError::SourceChanged);
        }
        checksum = history_chunk_checksum_update(checksum, &buffer[..read]);
        remaining = remaining.saturating_sub(read as u64);
    }
    let mut trailing = [0_u8; 1];
    if file
        .read(&mut trailing)
        .map_err(|_| HistoryRecordExportError::SourceChanged)?
        != 0
        || checksum != prepared.main_checksum
    {
        return Err(HistoryRecordExportError::SourceChanged);
    }
    Ok(())
}

fn write_chunked_history_export(
    prepared: &PreparedHistoryRecordExport,
    manifest: &HistoryDetailsChunkManifest,
    writer: &mut impl Write,
) -> Result<(), HistoryRecordExportError> {
    writer
        .write_all(b"{\"version\":")
        .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)?;
    write_history_export_value(writer, &prepared.version)?;
    writer
        .write_all(b",\"id\":")
        .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)?;
    write_history_export_value(writer, &prepared.id)?;
    writer
        .write_all(b",\"saved_at\":")
        .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)?;
    write_history_export_value(writer, &prepared.saved_at)?;
    if let Some(recorded_at) = &prepared.recorded_at {
        writer
            .write_all(b",\"recorded_at\":")
            .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)?;
        write_history_export_value(writer, recorded_at)?;
    }
    writer
        .write_all(b",\"summary\":")
        .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)?;
    write_history_export_value(writer, &prepared.summary)?;
    writer
        .write_all(b",\"details\":")
        .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)?;
    write_chunked_history_details(prepared, manifest, writer)?;
    writer
        .write_all(b"}\n")
        .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)
}

fn write_chunked_history_details(
    prepared: &PreparedHistoryRecordExport,
    manifest: &HistoryDetailsChunkManifest,
    writer: &mut impl Write,
) -> Result<(), HistoryRecordExportError> {
    let metadata = &manifest.metadata;
    writer
        .write_all(b"{\"floor\":")
        .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)?;
    write_history_export_value(writer, &metadata.floor)?;
    writer
        .write_all(b",\"active_half\":")
        .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)?;
    write_history_export_value(writer, &metadata.active_half)?;
    writer
        .write_all(b",\"first_half_at\":")
        .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)?;
    write_history_export_value(writer, &metadata.first_half_at)?;
    writer
        .write_all(b",\"second_half_at\":")
        .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)?;
    write_history_export_value(writer, &metadata.second_half_at)?;
    writer
        .write_all(b",\"success_at\":")
        .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)?;
    write_history_export_value(writer, &metadata.success_at)?;
    writer
        .write_all(b",\"exited_at\":")
        .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)?;
    write_history_export_value(writer, &metadata.exited_at)?;

    let directory = prepared
        .main_path
        .parent()
        .ok_or(HistoryRecordExportError::CorruptRecord)?;
    let mut loaded_hits = 0_u64;
    let mut loaded_bytes = 0_u64;
    for (field, lane) in [
        (b",\"global_hits\":".as_slice(), HistoryHitLane::Global),
        (
            b",\"first_half_hits\":".as_slice(),
            HistoryHitLane::FirstHalf,
        ),
        (
            b",\"second_half_hits\":".as_slice(),
            HistoryHitLane::SecondHalf,
        ),
    ] {
        writer
            .write_all(field)
            .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)?;
        write_history_hit_lane_export(
            directory,
            &prepared.id,
            manifest,
            lane,
            writer,
            &mut loaded_hits,
            &mut loaded_bytes,
        )?;
    }
    if loaded_hits != manifest.total_hits || loaded_bytes != manifest.total_bytes {
        return Err(HistoryRecordExportError::CorruptRecord);
    }
    writer
        .write_all(b",\"time_stop_events\":")
        .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)?;
    write_history_export_value(writer, &metadata.time_stop_events)?;
    writer
        .write_all(b",\"combat_clock_health\":")
        .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)?;
    write_history_export_value(writer, &metadata.combat_clock_health)?;
    writer
        .write_all(b"}")
        .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)
}

#[allow(clippy::too_many_arguments)]
fn write_history_hit_lane_export(
    directory: &Path,
    record_id: &str,
    manifest: &HistoryDetailsChunkManifest,
    lane: HistoryHitLane,
    writer: &mut impl Write,
    loaded_hits: &mut u64,
    loaded_bytes: &mut u64,
) -> Result<(), HistoryRecordExportError> {
    writer
        .write_all(b"[")
        .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)?;
    let mut first = true;
    for reference in manifest.chunks.iter().filter(|chunk| chunk.lane == lane) {
        let (chunk, bytes_read) = read_history_export_chunk(directory, record_id, reference)?;
        *loaded_hits = loaded_hits
            .checked_add(chunk.hits.len() as u64)
            .ok_or(HistoryRecordExportError::CorruptRecord)?;
        *loaded_bytes = loaded_bytes
            .checked_add(bytes_read)
            .ok_or(HistoryRecordExportError::CorruptRecord)?;
        for hit in &chunk.hits {
            if !first {
                writer
                    .write_all(b",")
                    .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)?;
            }
            write_history_export_value(writer, hit)?;
            first = false;
        }
    }
    writer
        .write_all(b"]")
        .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)
}

fn read_history_export_chunk(
    directory: &Path,
    record_id: &str,
    reference: &HistoryHitChunkRef,
) -> Result<(HistoryHitChunkRead, u64), HistoryRecordExportError> {
    let path = history_chunk_path(directory, record_id, reference.lane, reference.index);
    let file = source_file_for_history_export(&path, reference.bytes)?;
    let checksum_reader = HistoryChecksumReader::new(file.take(reference.bytes.saturating_add(1)));
    let mut reader = BufReader::with_capacity(64 * 1024, checksum_reader);
    let chunk: HistoryHitChunkRead = serde_json::from_reader(&mut reader)
        .map_err(|_| HistoryRecordExportError::CorruptRecord)?;
    let checksum_reader = reader.into_inner();
    if checksum_reader.bytes_read != reference.bytes
        || checksum_reader.checksum != reference.checksum
    {
        return Err(HistoryRecordExportError::CorruptRecord);
    }
    if chunk.version != HISTORY_DETAILS_CHUNK_VERSION
        || chunk.record_id != record_id
        || chunk.lane != reference.lane
        || chunk.index != reference.index
        || chunk.hits.len() != reference.hit_count
        || !history_hits_are_valid(&chunk.hits)
    {
        return Err(HistoryRecordExportError::CorruptRecord);
    }
    Ok((chunk, reference.bytes))
}

fn write_history_export_value(
    writer: &mut impl Write,
    value: &impl Serialize,
) -> Result<(), HistoryRecordExportError> {
    serde_json::to_writer(writer, value)
        .map_err(|_| HistoryRecordExportError::DestinationWriteFailed)
}

pub fn load_history_index() -> HistoryIndexLoadResult {
    load_history_index_from_dir(&history_dir())
}

pub fn load_history_index_from_dir(directory: &Path) -> HistoryIndexLoadResult {
    load_history_index_from_dir_with_limits(directory, HistoryDirectoryLimits::PRODUCTION)
}

fn load_history_index_from_dir_with_limits(
    directory: &Path,
    limits: HistoryDirectoryLimits,
) -> HistoryIndexLoadResult {
    let mut result = HistoryIndexLoadResult::default();
    let Ok(entries) = fs::read_dir(directory) else {
        return result;
    };
    let mut total_bytes = 0u64;
    let mut main_manifests = 0usize;
    for (entry_count, entry) in entries.enumerate() {
        if entry_count >= limits.max_entries {
            mark_history_file_skipped(&mut result.skipped_files);
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                mark_history_file_skipped(&mut result.skipped_files);
                continue;
            }
        };
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if main_manifests >= limits.max_main_manifests {
            mark_history_file_skipped(&mut result.skipped_files);
            break;
        }
        main_manifests = main_manifests.saturating_add(1);
        let Some(remaining_bytes) = limits.max_total_bytes.checked_sub(total_bytes) else {
            mark_history_file_skipped(&mut result.skipped_files);
            continue;
        };
        let parsed = history_candidate_size(&path, remaining_bytes)
            .map_err(|error| error.to_string())
            .and_then(|candidate_bytes| {
                // Charge the declared size before any read/parse attempt. Invalid
                // UTF-8/JSON therefore cannot bypass the aggregate I/O budget.
                total_bytes = total_bytes.saturating_add(candidate_bytes);
                read_history_text_with_limit(&path, candidate_bytes)
                    .map_err(|error| error.to_string())
                    .and_then(|(text, bytes)| parse_history_index(&text, &path, bytes))
            });
        match parsed {
            Ok(mut record) => {
                record.path = path;
                if insert_bounded_history_index(&mut result.records, record, limits.max_records) {
                    mark_history_file_skipped(&mut result.skipped_files);
                }
            }
            Err(_) => mark_history_file_skipped(&mut result.skipped_files),
        }
    }
    sort_history_index_newest_first(&mut result.records);
    result
}

fn sort_history_index_newest_first(records: &mut [HistoryIndexRecord]) {
    records.sort_by(|left, right| {
        right
            .effective_timestamp
            .cmp(&left.effective_timestamp)
            .then_with(|| right.id.cmp(&left.id))
    });
}

pub fn load_history_from_dir(directory: &Path) -> HistoryLoadResult {
    load_history_from_dir_with_limits(directory, HistoryDirectoryLimits::PRODUCTION)
}

pub fn load_history_summaries_from_dir(directory: &Path) -> HistoryLoadResult {
    load_history_summaries_from_dir_with_limits(directory, HistoryDirectoryLimits::PRODUCTION)
}

fn load_history_summaries_from_dir_with_limits(
    directory: &Path,
    limits: HistoryDirectoryLimits,
) -> HistoryLoadResult {
    let mut result = HistoryLoadResult::default();
    let Ok(entries) = fs::read_dir(directory) else {
        return result;
    };
    let mut total_bytes = 0_u64;
    let mut main_manifests = 0usize;
    for (entry_count, entry) in entries.enumerate() {
        if entry_count >= limits.max_entries {
            mark_history_file_skipped(&mut result.skipped_files);
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                mark_history_file_skipped(&mut result.skipped_files);
                continue;
            }
        };
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if main_manifests >= limits.max_main_manifests {
            mark_history_file_skipped(&mut result.skipped_files);
            break;
        }
        main_manifests = main_manifests.saturating_add(1);
        let Some(remaining_bytes) = limits.max_total_bytes.checked_sub(total_bytes) else {
            mark_history_file_skipped(&mut result.skipped_files);
            continue;
        };
        let parsed = history_candidate_size(&path, remaining_bytes)
            .map_err(|error| error.to_string())
            .and_then(|candidate_bytes| {
                total_bytes = total_bytes.saturating_add(candidate_bytes);
                read_history_text_with_limit(&path, candidate_bytes)
                    .map_err(|error| error.to_string())
                    .and_then(|(text, _)| parse_stored_history_summary(&text, &path))
            });
        match parsed {
            Ok(record) => {
                if insert_bounded_history_record(&mut result.records, record, limits.max_records) {
                    mark_history_file_skipped(&mut result.skipped_files);
                }
            }
            Err(_) => mark_history_file_skipped(&mut result.skipped_files),
        }
    }
    sort_records_newest_first(&mut result.records);
    result
}

fn load_history_from_dir_with_limits(
    directory: &Path,
    limits: HistoryDirectoryLimits,
) -> HistoryLoadResult {
    let mut result = HistoryLoadResult::default();
    let Ok(entries) = fs::read_dir(directory) else {
        return result;
    };
    let mut total_bytes = 0u64;
    let mut main_manifests = 0usize;
    for (entry_count, entry) in entries.enumerate() {
        if entry_count >= limits.max_entries {
            mark_history_file_skipped(&mut result.skipped_files);
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                mark_history_file_skipped(&mut result.skipped_files);
                continue;
            }
        };
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if main_manifests >= limits.max_main_manifests {
            mark_history_file_skipped(&mut result.skipped_files);
            break;
        }
        main_manifests = main_manifests.saturating_add(1);
        let Some(remaining_bytes) = limits.max_total_bytes.checked_sub(total_bytes) else {
            mark_history_file_skipped(&mut result.skipped_files);
            continue;
        };
        let parsed = history_candidate_size(&path, remaining_bytes)
            .map_err(|error| error.to_string())
            .and_then(|candidate_bytes| {
                total_bytes = total_bytes.saturating_add(candidate_bytes);
                read_history_text_with_limit(&path, candidate_bytes)
                    .map_err(|error| error.to_string())
                    .and_then(|(text, _)| {
                        let remaining = limits
                            .max_total_bytes
                            .checked_sub(total_bytes)
                            .ok_or_else(|| {
                                "History directory exceeds the supported scan budget".to_owned()
                            })?;
                        let (record, chunk_bytes) =
                            parse_stored_history_record(&text, &path, remaining)?;
                        total_bytes = total_bytes.saturating_add(chunk_bytes);
                        Ok(record)
                    })
            });
        match parsed {
            Ok(record) => {
                if insert_bounded_history_record(&mut result.records, record, limits.max_records) {
                    mark_history_file_skipped(&mut result.skipped_files);
                }
            }
            Err(_) => mark_history_file_skipped(&mut result.skipped_files),
        }
    }
    sort_records_newest_first(&mut result.records);
    result
}

fn read_history_text_with_limit(
    path: &Path,
    max_bytes: u64,
) -> Result<(String, u64), HistoryFileReadError> {
    let file = fs::File::open(path).map_err(|_| HistoryFileReadError::Read)?;
    let metadata = file
        .metadata()
        .map_err(|_| HistoryFileReadError::Metadata)?;
    if !metadata.is_file() {
        return Err(HistoryFileReadError::NotFile);
    }
    let max_bytes = max_bytes.min(MAX_HISTORY_IMPORT_BYTES);
    if metadata.len() > max_bytes {
        return Err(HistoryFileReadError::TooLarge);
    }
    let mut bytes = Vec::with_capacity(metadata.len().min(64 * 1024) as usize);
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| HistoryFileReadError::Read)?;
    if bytes.len() as u64 > max_bytes {
        return Err(HistoryFileReadError::TooLarge);
    }
    let bytes_read = bytes.len() as u64;
    let text = String::from_utf8(bytes).map_err(|_| HistoryFileReadError::Utf8)?;
    Ok((text, bytes_read))
}

fn history_candidate_size(path: &Path, remaining_bytes: u64) -> Result<u64, HistoryFileReadError> {
    let metadata = fs::metadata(path).map_err(|_| HistoryFileReadError::Metadata)?;
    if !metadata.is_file() {
        return Err(HistoryFileReadError::NotFile);
    }
    if metadata.len() > remaining_bytes.min(MAX_HISTORY_IMPORT_BYTES) {
        return Err(HistoryFileReadError::TooLarge);
    }
    Ok(metadata.len())
}

fn insert_bounded_history_record(
    records: &mut Vec<HistoryRecord>,
    record: HistoryRecord,
    limit: usize,
) -> bool {
    if limit == 0 {
        return true;
    }
    records.push(record);
    if records.len() <= limit {
        return false;
    }
    sort_records_newest_first(records);
    records.truncate(limit);
    true
}

fn insert_bounded_history_index(
    records: &mut Vec<HistoryIndexRecord>,
    record: HistoryIndexRecord,
    limit: usize,
) -> bool {
    if limit == 0 {
        return true;
    }
    records.push(record);
    if records.len() <= limit {
        return false;
    }
    sort_history_index_newest_first(records);
    records.truncate(limit);
    true
}

fn mark_history_file_skipped(skipped_files: &mut usize) {
    *skipped_files = skipped_files.saturating_add(1);
}

pub fn save_summary(summary: CombatSessionSummary) -> Result<HistoryRecord, String> {
    save_summary_to_dir(&history_dir(), summary)
}

pub fn save_summary_with_details(
    summary: CombatSessionSummary,
    details: HistoryCombatDetails,
) -> Result<HistoryRecord, String> {
    save_summary_with_details_to_dir(&history_dir(), summary, details)
}

/// Persists a prepared archive by reference. This is the retry-safe path used
/// by the desktop History owner: success drops the caller's archive, while a
/// pre-commit failure leaves the exact same owned hit vectors available for a
/// later retry without cloning them.
pub fn save_borrowed_archive_outcome(
    summary: &CombatSessionSummary,
    details: Option<&HistoryCombatDetails>,
) -> Result<BorrowedHistorySaveOutcome, HistorySaveError> {
    if let Some(details) = details {
        details
            .validate()
            .map_err(|_| HistorySaveError::InvalidDetails)?;
    }
    let directory = history_dir();
    let saved_at = Utc::now();
    let id = generate_record_id(saved_at);
    let recorded_at = details.and_then(HistoryCombatDetails::recorded_at);
    let warning = write_borrowed_record_to_dir_with_maintenance(
        &directory,
        HISTORY_RECORD_VERSION,
        &id,
        &saved_at,
        recorded_at.as_ref(),
        summary,
        details,
        |directory| prune_history_dir(directory, MAX_HISTORY_RECORDS),
    )?;
    Ok(match warning {
        Some(warning) => BorrowedHistorySaveOutcome::CommittedWithMaintenanceWarning(warning),
        None => BorrowedHistorySaveOutcome::Committed,
    })
}

pub fn import_record(path: &Path) -> Result<HistoryRecord, String> {
    import_record_to_dir(&history_dir(), path)
}

pub fn import_record_json(json: &str) -> Result<HistoryRecord, String> {
    import_record_json_to_dir(&history_dir(), json)
}

pub fn import_record_to_dir(directory: &Path, source_path: &Path) -> Result<HistoryRecord, String> {
    let (text, _) = read_history_text_with_limit(source_path, MAX_HISTORY_IMPORT_BYTES)
        .map_err(|error| error.to_string())?;
    import_record_text_to_dir(directory, &text, source_path)
}

pub fn import_record_json_to_dir(directory: &Path, json: &str) -> Result<HistoryRecord, String> {
    if json.len() as u64 > MAX_HISTORY_IMPORT_BYTES {
        return Err("History record exceeds the supported file size".to_owned());
    }
    import_record_text_to_dir(directory, json, Path::new("import.json"))
}

fn import_record_text_to_dir(
    directory: &Path,
    text: &str,
    source_path: &Path,
) -> Result<HistoryRecord, String> {
    let mut record = parse_history_record(text, source_path)?;
    record.version = HISTORY_RECORD_VERSION;
    record.id = generate_record_id(Utc::now());
    compatibility_save_result(write_record_to_dir_with_maintenance(
        directory,
        record,
        |directory| prune_history_dir(directory, MAX_HISTORY_RECORDS),
    ))
}

pub fn save_summary_to_dir(
    directory: &Path,
    summary: CombatSessionSummary,
) -> Result<HistoryRecord, String> {
    compatibility_save_result(save_summary_to_dir_outcome(directory, summary))
}

pub fn save_summary_with_details_to_dir(
    directory: &Path,
    summary: CombatSessionSummary,
    details: HistoryCombatDetails,
) -> Result<HistoryRecord, String> {
    details.validate()?;
    compatibility_save_result(save_record_to_dir(directory, summary, Some(details)))
}

pub fn save_summary_to_dir_outcome(
    directory: &Path,
    summary: CombatSessionSummary,
) -> Result<HistorySaveOutcome, HistorySaveError> {
    save_record_to_dir(directory, summary, None)
}

pub fn save_summary_with_details_to_dir_outcome(
    directory: &Path,
    summary: CombatSessionSummary,
    details: HistoryCombatDetails,
) -> Result<HistorySaveOutcome, HistorySaveError> {
    details
        .validate()
        .map_err(|_| HistorySaveError::InvalidDetails)?;
    save_record_to_dir(directory, summary, Some(details))
}

fn compatibility_save_result(
    result: Result<HistorySaveOutcome, HistorySaveError>,
) -> Result<HistoryRecord, String> {
    // Existing non-retrying command callers return only the record. Treat a
    // post-commit warning as success so a user action cannot create a second
    // ID after the first record was already made durable.
    result
        .map(HistorySaveOutcome::into_record)
        .map_err(|error| error.to_string())
}

fn save_record_to_dir(
    directory: &Path,
    summary: CombatSessionSummary,
    details: Option<HistoryCombatDetails>,
) -> Result<HistorySaveOutcome, HistorySaveError> {
    save_record_to_dir_with_maintenance(directory, summary, details, |directory| {
        prune_history_dir(directory, MAX_HISTORY_RECORDS)
    })
}

fn save_record_to_dir_with_maintenance(
    directory: &Path,
    summary: CombatSessionSummary,
    details: Option<HistoryCombatDetails>,
    maintain: impl FnOnce(&Path) -> Result<(), HistoryMaintenanceWarning>,
) -> Result<HistorySaveOutcome, HistorySaveError> {
    let saved_at = Utc::now();
    let id = generate_record_id(saved_at);
    let recorded_at = details.as_ref().and_then(HistoryCombatDetails::recorded_at);
    let record = HistoryRecord {
        version: HISTORY_RECORD_VERSION,
        id,
        saved_at,
        recorded_at,
        summary,
        details,
        details_chunks: None,
    };
    write_record_to_dir_with_maintenance(directory, record, maintain)
}

fn history_chunk_path(
    directory: &Path,
    record_id: &str,
    lane: HistoryHitLane,
    index: usize,
) -> PathBuf {
    directory.join(format!(
        "{record_id}.{}.{}.nte-history-chunk",
        lane.file_token(),
        format_args!("{index:06}")
    ))
}

fn history_main_path(directory: &Path, effective_timestamp: &DateTime<Utc>, id: &str) -> PathBuf {
    directory.join(format!(
        "{}_{}.json",
        effective_timestamp
            .with_timezone(&Local)
            .format("%Y%m%d_%H%M%S"),
        id
    ))
}

fn history_chunk_checksum(bytes: &[u8]) -> u64 {
    // Stable FNV-1a checksum. This is corruption/manifest binding rather than
    // an authentication primitive; paths are derived from the validated local
    // record id and never accepted from external JSON.
    history_chunk_checksum_update(0xcbf2_9ce4_8422_2325, bytes)
}

fn history_chunk_checksum_update(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn write_history_hit_lane(
    directory: &Path,
    record_id: &str,
    lane: HistoryHitLane,
    hits: &[Hit],
    references: &mut Vec<HistoryHitChunkRef>,
    created_paths: &mut Vec<PathBuf>,
    total_bytes: &mut u64,
) -> Result<(), HistorySaveError> {
    let mut start = 0_usize;
    let mut index = 0_usize;
    while start < hits.len() {
        if references.len() >= MAX_HISTORY_CHUNKS_PER_RECORD {
            return Err(HistorySaveError::TooLarge);
        }
        let mut end = start
            .saturating_add(MAX_HISTORY_HITS_PER_CHUNK)
            .min(hits.len());
        let bytes = loop {
            let document = HistoryHitChunkWrite {
                version: HISTORY_DETAILS_CHUNK_VERSION,
                record_id,
                lane,
                index,
                hits: &hits[start..end],
            };
            let bytes = serde_json::to_vec(&document).map_err(|_| HistorySaveError::Serialize)?;
            if bytes.len() as u64 <= MAX_HISTORY_CHUNK_BYTES {
                break bytes;
            }
            let count = end.saturating_sub(start);
            if count <= 1 {
                return Err(HistorySaveError::TooLarge);
            }
            end = start + count / 2;
        };
        *total_bytes = total_bytes
            .checked_add(bytes.len() as u64)
            .filter(|total| *total <= MAX_HISTORY_DETAILS_BYTES)
            .ok_or(HistorySaveError::TooLarge)?;
        let path = history_chunk_path(directory, record_id, lane, index);
        atomic_write_file(&path, |writer| {
            writer.write_all(&bytes).map_err(|error| error.to_string())
        })
        .map_err(|_| HistorySaveError::Commit)?;
        created_paths.push(path);
        references.push(HistoryHitChunkRef {
            lane,
            index,
            hit_count: end - start,
            bytes: bytes.len() as u64,
            checksum: history_chunk_checksum(&bytes),
        });
        start = end;
        index = index.saturating_add(1);
    }
    Ok(())
}

fn write_history_details_chunks(
    directory: &Path,
    record_id: &str,
    details: &HistoryCombatDetails,
) -> Result<(HistoryDetailsChunkManifest, Vec<PathBuf>), HistorySaveError> {
    let metadata = details.metadata_only();
    let mut references = Vec::new();
    let mut created_paths = Vec::new();
    let mut total_bytes = 0_u64;
    let result = (|| {
        for (lane, hits) in [
            (HistoryHitLane::Global, details.global_hits.as_slice()),
            (
                HistoryHitLane::FirstHalf,
                details.first_half_hits.as_slice(),
            ),
            (
                HistoryHitLane::SecondHalf,
                details.second_half_hits.as_slice(),
            ),
        ] {
            write_history_hit_lane(
                directory,
                record_id,
                lane,
                hits,
                &mut references,
                &mut created_paths,
                &mut total_bytes,
            )?;
        }
        let total_hits = details
            .global_hits
            .len()
            .saturating_add(details.first_half_hits.len())
            .saturating_add(details.second_half_hits.len()) as u64;
        Ok(HistoryDetailsChunkManifest {
            version: HISTORY_DETAILS_CHUNK_VERSION,
            metadata,
            chunks: references,
            total_hits,
            total_bytes,
        })
    })();
    if result.is_err() {
        for path in &created_paths {
            let _ = fs::remove_file(path);
        }
    }
    result.map(|manifest| (manifest, created_paths))
}

fn remove_chunk_paths(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn write_record_to_dir_with_maintenance(
    directory: &Path,
    mut record: HistoryRecord,
    maintain: impl FnOnce(&Path) -> Result<(), HistoryMaintenanceWarning>,
) -> Result<HistorySaveOutcome, HistorySaveError> {
    let warning = write_borrowed_record_to_dir_with_maintenance(
        directory,
        record.version,
        &record.id,
        &record.saved_at,
        record.recorded_at.as_ref(),
        &record.summary,
        record.details.as_ref(),
        maintain,
    )?;
    record.details_chunks = None;
    Ok(match warning {
        None => HistorySaveOutcome::Committed(record),
        Some(warning) => HistorySaveOutcome::CommittedWithMaintenanceWarning { record, warning },
    })
}

#[allow(clippy::too_many_arguments)]
fn write_borrowed_record_to_dir_with_maintenance(
    directory: &Path,
    version: u32,
    id: &str,
    saved_at: &DateTime<Utc>,
    recorded_at: Option<&DateTime<Utc>>,
    summary: &CombatSessionSummary,
    details: Option<&HistoryCombatDetails>,
    maintain: impl FnOnce(&Path) -> Result<(), HistoryMaintenanceWarning>,
) -> Result<Option<HistoryMaintenanceWarning>, HistorySaveError> {
    validate_history_summary(summary).map_err(|_| HistorySaveError::InvalidSummary)?;
    fs::create_dir_all(directory).map_err(|_| HistorySaveError::PrepareDirectory)?;
    let (manifest, created_paths) = if let Some(details) = details {
        let (manifest, paths) = write_history_details_chunks(directory, id, details)?;
        (Some(manifest), paths)
    } else {
        (None, Vec::new())
    };
    let disk_record = HistoryRecordDisk {
        version,
        id,
        saved_at,
        recorded_at,
        summary,
        details: None,
        details_chunks: manifest.as_ref(),
    };
    let text = match serde_json::to_string_pretty(&disk_record) {
        Ok(text) => text,
        Err(_) => {
            remove_chunk_paths(&created_paths);
            return Err(HistorySaveError::Serialize);
        }
    };
    if text.len() as u64 > MAX_HISTORY_IMPORT_BYTES {
        remove_chunk_paths(&created_paths);
        return Err(HistorySaveError::TooLarge);
    }
    let main_path = history_main_path(directory, recorded_at.unwrap_or(saved_at), id);
    if atomic_write_text(&main_path, &format!("{text}\n")).is_err() {
        remove_chunk_paths(&created_paths);
        return Err(HistorySaveError::Commit);
    }
    // `atomic_write_text` success is the commit boundary. Maintenance is a
    // separate effect and therefore cannot turn this operation back into a
    // precommit error.
    Ok(maintain(directory).err())
}

pub fn tombstone_record(
    record_id: &str,
) -> Result<Option<HistoryDeleteTombstone>, HistoryDeleteError> {
    tombstone_record_from_dir(&history_dir(), record_id)
}

pub fn restore_tombstoned_record(
    tombstone: &HistoryDeleteTombstone,
) -> Result<(), HistoryDeleteError> {
    restore_tombstoned_record_from_dir(tombstone)
}

pub fn discard_tombstoned_record(
    tombstone: &HistoryDeleteTombstone,
) -> Result<(), HistoryDeleteError> {
    discard_tombstoned_record_from_dir(tombstone)
}

pub fn restore_record_to_dir(directory: &Path, record: &HistoryRecord) -> Result<(), String> {
    if !valid_record_id(&record.id) {
        return Err("Invalid history record ID".to_owned());
    }
    // Undo must restore exactly the deleted record without evicting a different history entry.
    // A save performed during the undo window can temporarily put the directory one over the cap;
    // the next normal save applies the existing pruning policy.
    write_record_to_dir_with_maintenance(directory, record.clone(), |_| Ok(()))
        .map(HistorySaveOutcome::into_record)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub fn delete_record_from_dir(directory: &Path, record_id: &str) -> Result<bool, String> {
    delete_record_from_dir_with_limits(directory, record_id, HistoryDirectoryLimits::PRODUCTION)
}

fn delete_record_from_dir_with_limits(
    directory: &Path,
    record_id: &str,
    limits: HistoryDirectoryLimits,
) -> Result<bool, String> {
    let tombstone = tombstone_record_from_dir_with_limits(directory, record_id, limits)
        .map_err(|error| error.to_string())?;
    let Some(tombstone) = tombstone else {
        return Ok(false);
    };
    discard_tombstoned_record_from_dir(&tombstone).map_err(|error| error.to_string())?;
    Ok(true)
}

fn find_history_index_for_mutation(
    directory: &Path,
    record_id: &str,
    limits: HistoryDirectoryLimits,
) -> Result<Option<HistoryIndexRecord>, HistoryDeleteError> {
    if !valid_record_id(record_id) {
        return Err(HistoryDeleteError::InvalidId);
    }
    let entries = fs::read_dir(directory).map_err(|_| HistoryDeleteError::ScanFailed)?;
    let mut total_bytes = 0u64;
    let mut main_manifests = 0usize;
    let mut matched_record = None;
    for (entry_count, entry) in entries.enumerate() {
        if entry_count >= limits.max_entries {
            return Err(HistoryDeleteError::ScanBudgetExceeded);
        }
        let path = entry.map_err(|_| HistoryDeleteError::ScanFailed)?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if main_manifests >= limits.max_main_manifests {
            return Err(HistoryDeleteError::ScanBudgetExceeded);
        }
        main_manifests = main_manifests.saturating_add(1);
        let remaining_bytes = limits
            .max_total_bytes
            .checked_sub(total_bytes)
            .ok_or(HistoryDeleteError::ScanBudgetExceeded)?;
        let candidate_bytes = history_candidate_size(&path, remaining_bytes).map_err(|error| {
            if error == HistoryFileReadError::TooLarge {
                HistoryDeleteError::ScanBudgetExceeded
            } else {
                HistoryDeleteError::ScanFailed
            }
        })?;
        total_bytes = total_bytes.saturating_add(candidate_bytes);
        let (text, bytes) =
            read_history_text_with_limit(&path, candidate_bytes).map_err(|error| {
                if error == HistoryFileReadError::TooLarge {
                    HistoryDeleteError::ScanBudgetExceeded
                } else {
                    HistoryDeleteError::ScanFailed
                }
            })?;
        let record =
            parse_history_index(&text, &path, bytes).map_err(|_| HistoryDeleteError::ScanFailed)?;
        if record.id == record_id {
            if matched_record.is_some() {
                return Err(HistoryDeleteError::ScanFailed);
            }
            matched_record = Some(record);
        }
    }
    Ok(matched_record)
}

pub fn tombstone_record_from_dir(
    directory: &Path,
    record_id: &str,
) -> Result<Option<HistoryDeleteTombstone>, HistoryDeleteError> {
    tombstone_record_from_dir_with_limits(directory, record_id, HistoryDirectoryLimits::PRODUCTION)
}

fn tombstone_record_from_dir_with_limits(
    directory: &Path,
    record_id: &str,
    limits: HistoryDirectoryLimits,
) -> Result<Option<HistoryDeleteTombstone>, HistoryDeleteError> {
    let Some(index) = find_history_index_for_mutation(directory, record_id, limits)? else {
        return Ok(None);
    };
    let main_metadata =
        fs::symlink_metadata(&index.path).map_err(|_| HistoryDeleteError::ScanFailed)?;
    if main_metadata.file_type().is_symlink() || !main_metadata.is_file() {
        return Err(HistoryDeleteError::ScanFailed);
    }
    let main_expected_bytes = main_metadata.len();
    let (main_text, main_bytes) = read_history_text_with_limit(&index.path, main_expected_bytes)
        .map_err(|_| HistoryDeleteError::CorruptDetails)?;
    if main_bytes != main_expected_bytes {
        return Err(HistoryDeleteError::CorruptDetails);
    }
    let main_checksum = history_chunk_checksum(main_text.as_bytes());
    let mut chunk_files = Vec::with_capacity(index.detail_chunks.len());
    for chunk in &index.detail_chunks {
        let original = history_chunk_path(directory, record_id, chunk.lane, chunk.index);
        validate_history_tombstone_source(&original, chunk.bytes, chunk.checksum)?;
        chunk_files.push((original, chunk.bytes, chunk.checksum));
    }

    let (token, tombstone_directory) = prepare_history_tombstone_directory(directory)?;
    let owner = match create_history_tombstone_owner(&tombstone_directory, &token) {
        Ok(owner) => owner,
        Err(error) => {
            let _ = fs::remove_dir(&tombstone_directory);
            return Err(error);
        }
    };
    let main_name = index
        .path
        .file_name()
        .ok_or(HistoryDeleteError::PrepareFailed)?
        .to_owned();
    let main = HistoryTombstoneFile {
        original: index.path,
        tombstone: tombstone_directory.join(main_name),
        expected_bytes: main_expected_bytes,
        checksum: main_checksum,
    };
    let mut chunks = Vec::with_capacity(chunk_files.len());
    for (original, expected_bytes, checksum) in chunk_files {
        let name = original
            .file_name()
            .ok_or(HistoryDeleteError::PrepareFailed)?;
        chunks.push(HistoryTombstoneFile {
            tombstone: tombstone_directory.join(name),
            original,
            expected_bytes,
            checksum,
        });
    }

    // Main disappears first, so readers never discover a manifest after a
    // sidecar has moved. Any ordinary rename failure rolls every prior move
    // back before this function returns.
    let mut moved = Vec::with_capacity(chunks.len().saturating_add(1));
    if fs::rename(&main.original, &main.tombstone).is_err() {
        let _ = fs::remove_file(&owner.tombstone);
        let _ = fs::remove_dir(&tombstone_directory);
        return Err(HistoryDeleteError::MoveFailed);
    }
    moved.push(main.clone());
    for chunk in &chunks {
        if fs::rename(&chunk.original, &chunk.tombstone).is_err() {
            let rolled_back = rollback_history_tombstone_moves(&moved, false);
            let _ = fs::remove_file(&owner.tombstone);
            let _ = fs::remove_dir(&tombstone_directory);
            return Err(if rolled_back {
                HistoryDeleteError::MoveFailed
            } else {
                HistoryDeleteError::RollbackFailed
            });
        }
        moved.push(chunk.clone());
    }

    Ok(Some(HistoryDeleteTombstone {
        token,
        record_id: record_id.to_owned(),
        directory: tombstone_directory,
        owner,
        main,
        chunks,
    }))
}

fn create_history_tombstone_owner(
    directory: &Path,
    token: &str,
) -> Result<HistoryTombstoneFile, HistoryDeleteError> {
    let owner = HistoryTombstoneOwner {
        version: HISTORY_TOMBSTONE_OWNER_VERSION,
        token: token.to_owned(),
        owner_pid: std::process::id(),
        owner_started_at_ticks: process_start_identity(std::process::id()),
        created_at_millis: Utc::now().timestamp_millis(),
    };
    let text = serde_json::to_string(&owner).map_err(|_| HistoryDeleteError::PrepareFailed)?;
    let text = format!("{text}\n");
    if text.len() as u64 > MAX_HISTORY_TOMBSTONE_OWNER_BYTES {
        return Err(HistoryDeleteError::PrepareFailed);
    }
    let path = directory.join(HISTORY_TOMBSTONE_OWNER_FILE);
    atomic_write_text(&path, &text).map_err(|_| HistoryDeleteError::PrepareFailed)?;
    Ok(HistoryTombstoneFile {
        original: PathBuf::new(),
        tombstone: path,
        expected_bytes: text.len() as u64,
        checksum: history_chunk_checksum(text.as_bytes()),
    })
}

fn prepare_history_tombstone_directory(
    directory: &Path,
) -> Result<(String, PathBuf), HistoryDeleteError> {
    let root = directory.join(".nte-history-undo");
    match fs::symlink_metadata(&root) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(HistoryDeleteError::PrepareFailed);
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&root).map_err(|_| HistoryDeleteError::PrepareFailed)?;
        }
        Err(_) => return Err(HistoryDeleteError::PrepareFailed),
    }
    for _ in 0..16 {
        let counter = HISTORY_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        // Fixed-width local nonce + monotonic counter. The token is opaque at
        // the command boundary and never embeds the deleted record identity.
        let timestamp_nonce = (Utc::now().timestamp_nanos_opt().unwrap_or_default() as u64)
            .rotate_left(17)
            ^ u64::from(std::process::id()).rotate_left(41);
        let token = format!("undo-{timestamp_nonce:016x}{counter:016x}");
        let path = root.join(&token);
        match fs::create_dir(&path) {
            Ok(()) => return Ok((token, path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(HistoryDeleteError::PrepareFailed),
        }
    }
    Err(HistoryDeleteError::PrepareFailed)
}

fn validate_history_tombstone_source(
    path: &Path,
    expected_bytes: u64,
    checksum: u64,
) -> Result<(), HistoryDeleteError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| HistoryDeleteError::CorruptDetails)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != expected_bytes
    {
        return Err(HistoryDeleteError::CorruptDetails);
    }
    let (text, bytes) = read_history_text_with_limit(path, expected_bytes)
        .map_err(|_| HistoryDeleteError::CorruptDetails)?;
    if bytes != expected_bytes || history_chunk_checksum(text.as_bytes()) != checksum {
        return Err(HistoryDeleteError::CorruptDetails);
    }
    Ok(())
}

#[cfg(windows)]
fn process_start_identity(pid: u32) -> Option<u64> {
    // SAFETY: `OpenProcess` receives a numeric PID and the returned handle is
    // checked before four valid FILETIME out-pointers are passed to
    // `GetProcessTimes`. Every successful handle is closed exactly once.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: all pointers refer to initialized, writable FILETIME values and
    // `process` is a live owned handle until the subsequent CloseHandle call.
    let succeeded =
        unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) != 0 };
    // SAFETY: `process` was returned by OpenProcess and is not used again.
    let _ = unsafe { CloseHandle(process) };
    succeeded
        .then_some((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
}

#[cfg(not(windows))]
fn process_start_identity(pid: u32) -> Option<u64> {
    (pid == std::process::id()).then_some(0)
}

#[cfg(windows)]
fn tombstone_owner_status(owner: &HistoryTombstoneOwner) -> TombstoneOwnerStatus {
    // A matching process creation time prevents PID reuse from protecting a
    // dead owner's tombstone forever. Access-denied/other probe failures remain
    // Unknown and are fail-closed until the age bound is exceeded.
    // SAFETY: OpenProcess takes no pointers and its result is checked.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, owner.owner_pid) };
    if process.is_null() {
        // ERROR_INVALID_PARAMETER is the documented no-such-process result.
        // SAFETY: GetLastError reads thread-local Win32 state.
        return if unsafe { GetLastError() } == 87 {
            TombstoneOwnerStatus::Dead
        } else {
            TombstoneOwnerStatus::Unknown
        };
    }
    let current_start = process_start_identity(owner.owner_pid);
    // SAFETY: the extra probe handle is owned by this function.
    let _ = unsafe { CloseHandle(process) };
    match (owner.owner_started_at_ticks, current_start) {
        (Some(expected), Some(actual)) if expected == actual => TombstoneOwnerStatus::Active,
        (Some(_), Some(_)) => TombstoneOwnerStatus::Dead,
        _ => TombstoneOwnerStatus::Unknown,
    }
}

#[cfg(not(windows))]
fn tombstone_owner_status(owner: &HistoryTombstoneOwner) -> TombstoneOwnerStatus {
    if owner.owner_pid == std::process::id() {
        TombstoneOwnerStatus::Active
    } else {
        TombstoneOwnerStatus::Unknown
    }
}

fn rollback_history_tombstone_moves(files: &[HistoryTombstoneFile], restoring: bool) -> bool {
    let mut complete = true;
    for file in files.iter().rev() {
        let (from, to) = if restoring {
            (&file.original, &file.tombstone)
        } else {
            (&file.tombstone, &file.original)
        };
        complete &= fs::rename(from, to).is_ok();
    }
    complete
}

pub fn restore_tombstoned_record_from_dir(
    tombstone: &HistoryDeleteTombstone,
) -> Result<(), HistoryDeleteError> {
    validate_history_tombstone_source(
        &tombstone.owner.tombstone,
        tombstone.owner.expected_bytes,
        tombstone.owner.checksum,
    )?;
    for file in tombstone
        .chunks
        .iter()
        .chain(std::iter::once(&tombstone.main))
    {
        if fs::symlink_metadata(&file.original).is_ok() {
            return Err(HistoryDeleteError::RestoreConflict);
        }
        validate_history_tombstone_source(&file.tombstone, file.expected_bytes, file.checksum)?;
    }

    // Sidecars return first and the main manifest becomes visible last.
    let mut restored = Vec::with_capacity(tombstone.chunks.len().saturating_add(1));
    for file in tombstone
        .chunks
        .iter()
        .chain(std::iter::once(&tombstone.main))
    {
        if fs::rename(&file.tombstone, &file.original).is_err() {
            return Err(if rollback_history_tombstone_moves(&restored, true) {
                HistoryDeleteError::MoveFailed
            } else {
                HistoryDeleteError::RollbackFailed
            });
        }
        restored.push(file.clone());
    }
    let _ = fs::remove_file(&tombstone.owner.tombstone);
    let _ = fs::remove_dir(&tombstone.directory);
    if let Some(root) = tombstone.directory.parent() {
        let _ = fs::remove_dir(root);
    }
    Ok(())
}

pub fn discard_tombstoned_record_from_dir(
    tombstone: &HistoryDeleteTombstone,
) -> Result<(), HistoryDeleteError> {
    remove_history_tombstone_descriptor(tombstone)?;
    if let Some(root) = tombstone.directory.parent() {
        let _ = fs::remove_dir(root);
    }
    Ok(())
}

fn remove_history_tombstone_descriptor(
    tombstone: &HistoryDeleteTombstone,
) -> Result<(), HistoryDeleteError> {
    let metadata = match fs::symlink_metadata(&tombstone.directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(HistoryDeleteError::CleanupFailed),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(HistoryDeleteError::CleanupFailed);
    }
    let expected = tombstone
        .chunks
        .iter()
        .chain([&tombstone.main, &tombstone.owner])
        .collect::<Vec<_>>();
    let entries = fs::read_dir(&tombstone.directory)
        .map_err(|_| HistoryDeleteError::CleanupFailed)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| HistoryDeleteError::CleanupFailed)?;
    if entries.len() != expected.len() {
        return Err(HistoryDeleteError::CleanupFailed);
    }
    for entry in entries {
        let path = entry.path();
        if !expected.iter().any(|file| file.tombstone == path) {
            return Err(HistoryDeleteError::CleanupFailed);
        }
    }
    // Validate every declared file before deleting the first one. A replaced
    // directory, same-size mutation, symlink, missing file, or extra entry is
    // therefore fail-closed and leaves the tombstone available for inspection.
    for file in &expected {
        validate_history_tombstone_source(&file.tombstone, file.expected_bytes, file.checksum)
            .map_err(|_| HistoryDeleteError::CleanupFailed)?;
    }
    for file in expected {
        fs::remove_file(&file.tombstone).map_err(|_| HistoryDeleteError::CleanupFailed)?;
    }
    fs::remove_dir(&tombstone.directory).map_err(|_| HistoryDeleteError::CleanupFailed)
}

/// Bounded startup/maintenance cleanup for tombstones whose in-memory undo
/// owner disappeared. No recursive traversal or symlink following is used.
pub fn cleanup_orphaned_history_tombstones(directory: &Path) -> Result<usize, HistoryDeleteError> {
    cleanup_orphaned_history_tombstones_with(
        directory,
        Utc::now().timestamp_millis(),
        tombstone_owner_status,
    )
}

fn cleanup_orphaned_history_tombstones_with(
    directory: &Path,
    now_millis: i64,
    mut owner_status: impl FnMut(&HistoryTombstoneOwner) -> TombstoneOwnerStatus,
) -> Result<usize, HistoryDeleteError> {
    let root = directory.join(".nte-history-undo");
    let metadata = match fs::symlink_metadata(&root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(_) => return Err(HistoryDeleteError::CleanupFailed),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(HistoryDeleteError::CleanupFailed);
    }
    let entries = fs::read_dir(&root).map_err(|_| HistoryDeleteError::CleanupFailed)?;
    let mut removed = 0_usize;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_HISTORY_RECORDS.saturating_mul(2) {
            return Err(HistoryDeleteError::CleanupFailed);
        }
        let path = entry.map_err(|_| HistoryDeleteError::CleanupFailed)?.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(HistoryDeleteError::CleanupFailed)?;
        if name.len() != 37
            || !name.starts_with("undo-")
            || !name[5..].bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(HistoryDeleteError::CleanupFailed);
        }
        let owner = read_history_tombstone_owner(&path, name)?;
        let expired = now_millis
            .checked_sub(owner.created_at_millis)
            .is_some_and(|age| age >= MAX_HISTORY_TOMBSTONE_AGE_MILLIS);
        if !expired && owner_status(&owner) != TombstoneOwnerStatus::Dead {
            continue;
        }
        remove_history_tombstone_directory(&path)?;
        removed = removed.saturating_add(1);
    }
    let _ = fs::remove_dir(root);
    Ok(removed)
}

fn read_history_tombstone_owner(
    directory: &Path,
    expected_token: &str,
) -> Result<HistoryTombstoneOwner, HistoryDeleteError> {
    let path = directory.join(HISTORY_TOMBSTONE_OWNER_FILE);
    let metadata = fs::symlink_metadata(&path).map_err(|_| HistoryDeleteError::CleanupFailed)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_HISTORY_TOMBSTONE_OWNER_BYTES
    {
        return Err(HistoryDeleteError::CleanupFailed);
    }
    let (text, bytes) = read_history_text_with_limit(&path, metadata.len())
        .map_err(|_| HistoryDeleteError::CleanupFailed)?;
    if bytes != metadata.len() {
        return Err(HistoryDeleteError::CleanupFailed);
    }
    let owner: HistoryTombstoneOwner =
        serde_json::from_str(&text).map_err(|_| HistoryDeleteError::CleanupFailed)?;
    if owner.version != HISTORY_TOMBSTONE_OWNER_VERSION
        || owner.token != expected_token
        || owner.owner_pid == 0
        || owner.created_at_millis <= 0
    {
        return Err(HistoryDeleteError::CleanupFailed);
    }
    Ok(owner)
}

/// Runs the bounded orphan cleanup against the application History directory.
/// The desktop History owner calls this once during startup, before it can own
/// an active in-memory undo token.
pub fn cleanup_orphaned_history_tombstones_at_startup() -> Result<usize, HistoryDeleteError> {
    cleanup_orphaned_history_tombstones(&history_dir())
}

fn remove_history_tombstone_directory(path: &Path) -> Result<(), HistoryDeleteError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(HistoryDeleteError::CleanupFailed),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(HistoryDeleteError::CleanupFailed);
    }
    let entries = fs::read_dir(path).map_err(|_| HistoryDeleteError::CleanupFailed)?;
    let mut files = Vec::new();
    for (index, entry) in entries.enumerate() {
        if index >= MAX_HISTORY_TOMBSTONE_FILES_PER_RECORD {
            return Err(HistoryDeleteError::CleanupFailed);
        }
        let path = entry.map_err(|_| HistoryDeleteError::CleanupFailed)?.path();
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| HistoryDeleteError::CleanupFailed)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(HistoryDeleteError::CleanupFailed);
        }
        files.push(path);
    }
    for file in files {
        fs::remove_file(file).map_err(|_| HistoryDeleteError::CleanupFailed)?;
    }
    fs::remove_dir(path).map_err(|_| HistoryDeleteError::CleanupFailed)
}

pub fn compare_records(left: &HistoryRecord, right: &HistoryRecord) -> HistoryComparison {
    let mut character_deltas =
        compare_characters(&left.summary.characters, &right.summary.characters);
    character_deltas.sort_by(|left, right| {
        right
            .delta_damage
            .abs()
            .total_cmp(&left.delta_damage.abs())
            .then_with(|| left.name.cmp(&right.name))
    });
    character_deltas.truncate(8);

    let mut skill_deltas = compare_skills(&left.summary.skills, &right.summary.skills);
    skill_deltas.sort_by(|left, right| {
        right
            .delta_damage
            .abs()
            .total_cmp(&left.delta_damage.abs())
            .then_with(|| left.name.cmp(&right.name))
    });
    skill_deltas.truncate(8);

    HistoryComparison {
        left_id: left.id.clone(),
        right_id: right.id.clone(),
        total_dps_delta: right.summary.total_dps - left.summary.total_dps,
        total_damage_delta: right.summary.total_damage - left.summary.total_damage,
        duration_delta: right.summary.duration_seconds - left.summary.duration_seconds,
        character_deltas,
        skill_deltas,
    }
}

fn validate_history_summary(summary: &CombatSessionSummary) -> Result<(), String> {
    validate_history_summary_scope(
        summary.duration_seconds,
        summary.total_damage,
        summary.total_dps,
        &summary.damage_attribution,
        &summary.characters,
        &summary.skills,
    )?;
    validate_nonnegative_finite(summary.total_damage_taken, "total damage taken")?;
    validate_capture_quality_summary(&summary.quality)?;

    let mut character_rows = summary.characters.len();
    let mut skill_rows = summary.skills.len();
    for half in [
        summary.abyss.first_half.as_ref(),
        summary.abyss.second_half.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        validate_history_half_summary(half)?;
        character_rows = character_rows
            .checked_add(half.characters.len())
            .ok_or_else(|| "History summary character count overflow".to_owned())?;
        skill_rows = skill_rows
            .checked_add(half.skills.len())
            .ok_or_else(|| "History summary skill count overflow".to_owned())?;
    }
    if character_rows
        > MAX_HISTORY_SUMMARY_CHARACTERS_PER_SCOPE.saturating_mul(MAX_HISTORY_SUMMARY_SCOPES)
        || skill_rows
            > MAX_HISTORY_SUMMARY_SKILLS_PER_SCOPE.saturating_mul(MAX_HISTORY_SUMMARY_SCOPES)
    {
        return Err("History summary nested row count exceeds the storage limit".to_owned());
    }
    Ok(())
}

fn validate_history_half_summary(half: &CombatSessionAbyssHalfSummary) -> Result<(), String> {
    validate_history_summary_scope(
        half.duration_seconds,
        half.total_damage,
        half.total_dps,
        &half.damage_attribution,
        &half.characters,
        &half.skills,
    )
}

fn validate_history_summary_scope(
    duration_seconds: f64,
    total_damage: f64,
    total_dps: f64,
    attribution: &DamageAttributionSummary,
    characters: &[CombatSessionCharacterSummary],
    skills: &[CombatSessionSkillSummary],
) -> Result<(), String> {
    if characters.len() > MAX_HISTORY_SUMMARY_CHARACTERS_PER_SCOPE
        || skills.len() > MAX_HISTORY_SUMMARY_SKILLS_PER_SCOPE
    {
        return Err("History summary row count exceeds the storage limit".to_owned());
    }
    validate_nonnegative_finite(duration_seconds, "duration")?;
    validate_nonnegative_finite(total_damage, "total damage")?;
    validate_nonnegative_finite(total_dps, "total DPS")?;
    validate_damage_attribution_summary(attribution)?;
    for row in characters {
        validate_history_summary_text(&row.name)?;
        validate_nonnegative_finite(row.damage, "character damage")?;
        validate_nonnegative_finite(row.dps, "character DPS")?;
        validate_percentage(row.damage_share_percent)?;
        validate_nonnegative_finite(row.damage_taken, "character damage taken")?;
    }
    for row in skills {
        for value in [
            Some(row.char_name.as_str()),
            Some(row.name.as_str()),
            Some(row.category.as_str()),
            row.ability_name.as_deref(),
            row.gameplay_effect_name.as_deref(),
            row.damage_name.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            validate_history_summary_text(value)?;
        }
        validate_nonnegative_finite(row.damage, "skill damage")?;
        validate_percentage(row.damage_share_percent)?;
    }
    Ok(())
}

fn validate_damage_attribution_summary(summary: &DamageAttributionSummary) -> Result<(), String> {
    for value in [
        summary.total_damage,
        summary.max_hp_reduction,
        summary.character_direct_damage,
        summary.character_reaction_damage,
        summary.shared_damage,
        summary.unattributed_damage,
    ] {
        validate_nonnegative_finite(value, "damage attribution")?;
    }
    Ok(())
}

fn validate_capture_quality_summary(summary: &CaptureQualitySummary) -> Result<(), String> {
    for value in [
        summary.outgoing_damage,
        summary.unknown_direction_damage,
        summary.incoming_damage,
        summary.unattributed_server_damage,
    ] {
        validate_nonnegative_finite(value, "capture quality damage")?;
    }
    Ok(())
}

fn validate_history_summary_text(value: &str) -> Result<(), String> {
    if value.len() > MAX_HISTORY_SUMMARY_TEXT_BYTES {
        return Err("History summary text exceeds the storage limit".to_owned());
    }
    Ok(())
}

fn validate_nonnegative_finite(value: f64, field: &str) -> Result<(), String> {
    if !value.is_finite() || value < 0.0 {
        return Err(format!("History summary contains invalid {field}"));
    }
    Ok(())
}

fn validate_percentage(value: f64) -> Result<(), String> {
    if !value.is_finite() || !(0.0..=100.000_001).contains(&value) {
        return Err("History summary contains an invalid percentage".to_owned());
    }
    Ok(())
}

fn parse_history_envelope(text: &str, path: &Path) -> Result<HistoryRecord, String> {
    let mut record: HistoryRecord =
        serde_json::from_str(text).map_err(|error| error.to_string())?;
    if record.version > HISTORY_RECORD_VERSION {
        return Err(format!("Unsupported history version {}", record.version));
    }
    if record.id.trim().is_empty() {
        record.id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("legacy")
            .to_owned();
    }
    if !valid_record_id(&record.id) {
        return Err("Invalid history record ID".to_owned());
    }
    validate_history_summary(&record.summary)?;
    Ok(record)
}

fn parse_history_record(text: &str, path: &Path) -> Result<HistoryRecord, String> {
    let record = parse_history_envelope(text, path)?;
    if record.details_chunks.is_some() {
        return Err("External History records cannot reference local detail chunks".to_owned());
    }
    if let Some(details) = &record.details {
        details.validate_external()?;
    }
    Ok(record)
}

fn history_lane_slot(lane: HistoryHitLane) -> usize {
    match lane {
        HistoryHitLane::Global => 0,
        HistoryHitLane::FirstHalf => 1,
        HistoryHitLane::SecondHalf => 2,
    }
}

fn history_chunk_manifest_is_valid(manifest: &HistoryDetailsChunkManifest) -> bool {
    if manifest.version != HISTORY_DETAILS_CHUNK_VERSION
        || !manifest.metadata.global_hits.is_empty()
        || !manifest.metadata.first_half_hits.is_empty()
        || !manifest.metadata.second_half_hits.is_empty()
        || manifest.metadata.time_stop_events.len() > MAX_HISTORY_IMPORT_TIME_STOP_EVENTS
    {
        return false;
    }
    history_chunk_lanes_are_valid(&manifest.chunks)
        && history_chunk_references_are_valid(
            &manifest.chunks,
            manifest.total_hits,
            manifest.total_bytes,
        )
}

fn history_chunk_manifest_index_is_valid(manifest: &HistoryDetailsChunkManifestIndex) -> bool {
    manifest.version == HISTORY_DETAILS_CHUNK_VERSION
        && manifest.metadata.is_some()
        && history_chunk_lanes_are_valid(&manifest.chunks)
        && history_chunk_references_are_valid(
            &manifest.chunks,
            manifest.total_hits,
            manifest.total_bytes,
        )
}

fn history_chunk_lanes_are_valid(chunks: &[HistoryHitChunkRef]) -> bool {
    let has_global = chunks
        .iter()
        .any(|chunk| chunk.lane == HistoryHitLane::Global);
    let has_abyss = chunks.iter().any(|chunk| {
        matches!(
            chunk.lane,
            HistoryHitLane::FirstHalf | HistoryHitLane::SecondHalf
        )
    });
    !(has_global && has_abyss)
}

fn history_chunk_references_are_valid(
    chunks: &[HistoryHitChunkRef],
    total_hits: u64,
    total_bytes: u64,
) -> bool {
    if chunks.len() > MAX_HISTORY_CHUNKS_PER_RECORD || total_bytes > MAX_HISTORY_DETAILS_BYTES {
        return false;
    }
    let mut expected_indices = [0_usize; 3];
    let mut bytes = 0_u64;
    let mut hits = 0_u64;
    for chunk in chunks {
        let slot = history_lane_slot(chunk.lane);
        if chunk.index != expected_indices[slot]
            || chunk.hit_count == 0
            || chunk.hit_count > MAX_HISTORY_HITS_PER_CHUNK
            || chunk.bytes == 0
            || chunk.bytes > MAX_HISTORY_CHUNK_BYTES
        {
            return false;
        }
        expected_indices[slot] = expected_indices[slot].saturating_add(1);
        let Some(next_bytes) = bytes.checked_add(chunk.bytes) else {
            return false;
        };
        let Some(next_hits) = hits.checked_add(chunk.hit_count as u64) else {
            return false;
        };
        bytes = next_bytes;
        hits = next_hits;
    }
    bytes == total_bytes && hits == total_hits
}

fn load_history_details_chunks(
    directory: &Path,
    record_id: &str,
    manifest: HistoryDetailsChunkManifest,
    max_total_bytes: u64,
) -> Result<(HistoryCombatDetails, u64), String> {
    if !history_chunk_manifest_is_valid(&manifest) || manifest.total_bytes > max_total_bytes {
        return Err("History detail chunk manifest is invalid".to_owned());
    }

    let mut details = manifest.metadata;
    let mut expected_indices = [0_usize; 3];
    let mut loaded_hits = 0_u64;
    let mut loaded_bytes = 0_u64;
    for reference in manifest.chunks {
        let slot = history_lane_slot(reference.lane);
        if reference.index != expected_indices[slot] {
            return Err("History detail chunk sequence is invalid".to_owned());
        }
        expected_indices[slot] = expected_indices[slot].saturating_add(1);
        let path = history_chunk_path(directory, record_id, reference.lane, reference.index);
        let (text, bytes_read) = read_history_text_with_limit(&path, reference.bytes)
            .map_err(|error| error.to_string())?;
        if bytes_read != reference.bytes
            || history_chunk_checksum(text.as_bytes()) != reference.checksum
        {
            return Err("History detail chunk integrity check failed".to_owned());
        }
        let chunk: HistoryHitChunkRead =
            serde_json::from_str(&text).map_err(|error| error.to_string())?;
        if chunk.version != HISTORY_DETAILS_CHUNK_VERSION
            || chunk.record_id != record_id
            || chunk.lane != reference.lane
            || chunk.index != reference.index
            || chunk.hits.len() != reference.hit_count
        {
            return Err("History detail chunk contents do not match the manifest".to_owned());
        }
        loaded_hits = loaded_hits
            .checked_add(chunk.hits.len() as u64)
            .ok_or_else(|| "History detail hit count overflow".to_owned())?;
        loaded_bytes = loaded_bytes
            .checked_add(bytes_read)
            .ok_or_else(|| "History detail byte count overflow".to_owned())?;
        match reference.lane {
            HistoryHitLane::Global => details.global_hits.extend(chunk.hits),
            HistoryHitLane::FirstHalf => details.first_half_hits.extend(chunk.hits),
            HistoryHitLane::SecondHalf => details.second_half_hits.extend(chunk.hits),
        }
    }
    if loaded_hits != manifest.total_hits || loaded_bytes != manifest.total_bytes {
        return Err("History detail chunk totals do not match the manifest".to_owned());
    }
    details.validate()?;
    Ok((details, loaded_bytes))
}

fn parse_stored_history_record(
    text: &str,
    path: &Path,
    max_chunk_bytes: u64,
) -> Result<(HistoryRecord, u64), String> {
    let mut record = parse_history_envelope(text, path)?;
    match (record.details.as_ref(), record.details_chunks.take()) {
        (Some(_), Some(_)) => Err("History record mixes inline and chunked details".to_owned()),
        (Some(details), None) => {
            details.validate_external()?;
            Ok((record, 0))
        }
        (None, Some(manifest)) => {
            let directory = path
                .parent()
                .ok_or_else(|| "History record has no storage directory".to_owned())?;
            let (details, bytes) =
                load_history_details_chunks(directory, &record.id, manifest, max_chunk_bytes)?;
            record.details = Some(details);
            Ok((record, bytes))
        }
        (None, None) => Ok((record, 0)),
    }
}

fn parse_stored_history_summary(text: &str, path: &Path) -> Result<HistoryRecord, String> {
    let envelope: HistorySummaryEnvelope =
        serde_json::from_str(text).map_err(|error| error.to_string())?;
    if envelope.version > HISTORY_RECORD_VERSION {
        return Err(format!("Unsupported history version {}", envelope.version));
    }
    let id = if envelope.id.trim().is_empty() {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("legacy")
            .to_owned()
    } else {
        envelope.id
    };
    if !valid_record_id(&id) {
        return Err("Invalid history record ID".to_owned());
    }
    validate_history_summary(&envelope.summary)?;
    if envelope.details.is_some() && envelope.details_chunks.is_some() {
        return Err("History record mixes inline and chunked details".to_owned());
    }
    if envelope
        .details_chunks
        .as_ref()
        .is_some_and(|manifest| !history_chunk_manifest_index_is_valid(manifest))
    {
        return Err("History detail chunk manifest is invalid".to_owned());
    }
    let has_details = envelope.details.is_some() || envelope.details_chunks.is_some();
    Ok(HistoryRecord {
        version: envelope.version,
        id,
        saved_at: envelope.saved_at,
        recorded_at: envelope.recorded_at,
        summary: envelope.summary,
        details: has_details.then(HistoryCombatDetails::default),
        details_chunks: None,
    })
}

fn parse_history_index(
    text: &str,
    path: &Path,
    main_file_bytes: u64,
) -> Result<HistoryIndexRecord, String> {
    let record: HistoryIndexEnvelope =
        serde_json::from_str(text).map_err(|error| error.to_string())?;
    if record.version > HISTORY_RECORD_VERSION {
        return Err(format!("Unsupported history version {}", record.version));
    }
    let id = if record.id.trim().is_empty() {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("legacy")
            .to_owned()
    } else {
        record.id
    };
    if !valid_record_id(&id) {
        return Err("Invalid history record ID".to_owned());
    }
    if record
        .details_chunks
        .as_ref()
        .is_some_and(|manifest| !history_chunk_manifest_index_is_valid(manifest))
    {
        return Err("History detail chunk manifest is invalid".to_owned());
    }
    let effective_timestamp = record.recorded_at.unwrap_or(record.saved_at);
    let detail_chunks = record
        .details_chunks
        .as_ref()
        .map_or_else(Vec::new, |manifest| manifest.chunks.clone());
    let stored_detail_bytes = record.details_chunks.as_ref().map_or_else(
        || u64::from(record.details.is_some()) * main_file_bytes,
        |manifest| manifest.total_bytes,
    );
    let total_detail_hits = record
        .details_chunks
        .as_ref()
        .map(|manifest| manifest.total_hits);
    Ok(HistoryIndexRecord {
        path: path.to_owned(),
        id,
        display_time: effective_timestamp
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
        abyss_floor: record.summary.abyss.floor,
        has_details: record.details.is_some() || record.details_chunks.is_some(),
        stored_detail_bytes,
        total_detail_hits,
        effective_timestamp,
        detail_chunks,
    })
}

fn valid_record_id(record_id: &str) -> bool {
    !record_id.is_empty()
        && record_id.len() <= 128
        && record_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn prune_history_dir(
    directory: &Path,
    max_records: usize,
) -> Result<(), HistoryMaintenanceWarning> {
    prune_history_dir_with_entry_limit(
        directory,
        max_records,
        MAX_HISTORY_DIRECTORY_ENTRIES,
        MAX_HISTORY_MAIN_MANIFEST_CANDIDATES,
    )
}

fn prune_history_dir_with_entry_limit(
    directory: &Path,
    max_records: usize,
    max_entries: usize,
    max_main_manifests: usize,
) -> Result<(), HistoryMaintenanceWarning> {
    let entries =
        fs::read_dir(directory).map_err(|_| HistoryMaintenanceWarning::RetentionPruneFailed)?;
    let mut files = Vec::with_capacity(max_entries.min(max_records.saturating_add(1)));
    let mut main_manifests = 0usize;
    for (entry_count, entry) in entries.enumerate() {
        if entry_count >= max_entries {
            return Err(HistoryMaintenanceWarning::RetentionPruneFailed);
        }
        let path = entry
            .map_err(|_| HistoryMaintenanceWarning::RetentionPruneFailed)?
            .path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if main_manifests >= max_main_manifests {
            return Err(HistoryMaintenanceWarning::RetentionPruneFailed);
        }
        main_manifests = main_manifests.saturating_add(1);
        let modified = fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .map_err(|_| HistoryMaintenanceWarning::RetentionPruneFailed)?;
        files.push((modified, path));
    }
    files.sort_by_key(|(modified, _)| *modified);
    let remove_count = files.len().saturating_sub(max_records);
    for (_, path) in files.into_iter().take(remove_count) {
        let (text, bytes) = read_history_text_with_limit(&path, MAX_HISTORY_IMPORT_BYTES)
            .map_err(|_| HistoryMaintenanceWarning::RetentionPruneFailed)?;
        let index = parse_history_index(&text, &path, bytes)
            .map_err(|_| HistoryMaintenanceWarning::RetentionPruneFailed)?;
        fs::remove_file(&path).map_err(|_| HistoryMaintenanceWarning::RetentionPruneFailed)?;
        for chunk in index.detail_chunks {
            let chunk_path = history_chunk_path(directory, &index.id, chunk.lane, chunk.index);
            if let Err(error) = fs::remove_file(chunk_path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                return Err(HistoryMaintenanceWarning::RetentionPruneFailed);
            }
        }
    }
    Ok(())
}

fn sort_records_newest_first(records: &mut [HistoryRecord]) {
    records.sort_by(|left, right| {
        right
            .effective_timestamp()
            .cmp(left.effective_timestamp())
            .then_with(|| right.id.cmp(&left.id))
    });
}

fn generate_record_id(saved_at: DateTime<Utc>) -> String {
    let counter = HISTORY_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = saved_at
        .timestamp_nanos_opt()
        .unwrap_or_else(|| saved_at.timestamp_millis().saturating_mul(1_000_000));
    let mut value = (nanos as u64) ^ ((std::process::id() as u64) << 32) ^ counter;
    value ^= value << 13;
    value ^= value >> 7;
    value ^= value << 17;
    format!("{:08x}", value & 0xffff_ffff)
}

fn compare_characters(
    left: &[CombatSessionCharacterSummary],
    right: &[CombatSessionCharacterSummary],
) -> Vec<HistoryCharacterDelta> {
    let mut rows = std::collections::HashMap::<u32, HistoryCharacterDelta>::new();
    for row in left {
        rows.insert(
            row.char_id,
            HistoryCharacterDelta {
                char_id: row.char_id,
                name: row.name.clone(),
                left_dps: row.dps,
                left_damage: row.damage,
                ..Default::default()
            },
        );
    }
    for row in right {
        let entry = rows
            .entry(row.char_id)
            .or_insert_with(|| HistoryCharacterDelta {
                char_id: row.char_id,
                name: row.name.clone(),
                ..Default::default()
            });
        if entry.name.is_empty() {
            entry.name.clone_from(&row.name);
        }
        entry.right_dps = row.dps;
        entry.right_damage = row.damage;
    }
    for row in rows.values_mut() {
        row.delta_dps = row.right_dps - row.left_dps;
        row.delta_damage = row.right_damage - row.left_damage;
    }
    rows.into_values().collect()
}

fn compare_skills(
    left: &[CombatSessionSkillSummary],
    right: &[CombatSessionSkillSummary],
) -> Vec<HistorySkillDelta> {
    let mut rows = std::collections::HashMap::<(String, String), HistorySkillDelta>::new();
    for row in left {
        let comparison_name = row.damage_name.as_ref().unwrap_or(&row.name);
        let key = skill_comparison_key(row, right);
        let entry = rows.entry(key).or_insert_with(|| HistorySkillDelta {
            name: comparison_name.clone(),
            category: row.category.clone(),
            ..Default::default()
        });
        preserve_skill_delta_identity(entry, row);
        entry.left_damage += row.damage;
    }
    for row in right {
        let comparison_name = row.damage_name.as_ref().unwrap_or(&row.name);
        let key = skill_comparison_key(row, left);
        let entry = rows.entry(key).or_insert_with(|| HistorySkillDelta {
            name: comparison_name.clone(),
            category: row.category.clone(),
            ..Default::default()
        });
        preserve_skill_delta_identity(entry, row);
        entry.right_damage += row.damage;
    }
    for row in rows.values_mut() {
        row.delta_damage = row.right_damage - row.left_damage;
    }
    rows.into_values().collect()
}

fn preserve_skill_delta_identity(
    delta: &mut HistorySkillDelta,
    summary: &CombatSessionSkillSummary,
) {
    if delta.ability_name.is_none() {
        delta.ability_name.clone_from(&summary.ability_name);
    }
    if delta.gameplay_effect_name.is_none() {
        delta
            .gameplay_effect_name
            .clone_from(&summary.gameplay_effect_name);
    }
}

fn skill_comparison_key(
    row: &CombatSessionSkillSummary,
    other: &[CombatSessionSkillSummary],
) -> (String, String) {
    if row.ability_name.is_none() && row.gameplay_effect_name.is_none() {
        return (format!("legacy:{}", row.name), row.category.clone());
    }
    if let Some(display_name) = row.damage_name.as_deref()
        && other.iter().any(|candidate| {
            candidate.category == row.category
                && candidate.ability_name.is_none()
                && candidate.gameplay_effect_name.is_none()
                && candidate.name == display_name
        })
    {
        return (format!("legacy:{display_name}"), row.category.clone());
    }
    let identity = match row.ability_name.as_deref() {
        Some(ability_name) => {
            if let Some(component_name) = row
                .damage_name
                .as_deref()
                .filter(|damage_name| *damage_name == row.name && *damage_name != ability_name)
            {
                format!("ability:{ability_name}:component:{component_name}")
            } else {
                format!("ability:{ability_name}")
            }
        }
        None => format!(
            "effect:{}",
            row.gameplay_effect_name.as_deref().unwrap_or("unknown")
        ),
    };
    (format!("stable:{identity}"), row.category.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::model::{
        AbyssHalf, CombatSessionAbyssHalfSummary, CombatSessionAbyssSummary,
        CombatSessionCharacterSummary, CombatSessionSummary, DpsTimeBasis, HitCharacterSource,
        HitDirection,
    };

    #[test]
    fn directory_loaders_keep_only_the_newest_bounded_records() {
        let directory = temp_history_dir("bounded_newest");
        fs::create_dir_all(&directory).expect("create bounded History directory");
        for index in 0..(MAX_HISTORY_RECORDS + 5) {
            let timestamp = format!("2026-01-01T00:{:02}:{:02}Z", index / 60, index % 60);
            fs::write(
                directory.join(format!("record-{index:03}.json")),
                format!(
                    r#"{{"version":1,"id":"record-{index:03}","saved_at":"{timestamp}","summary":{{}}}}"#
                ),
            )
            .expect("write bounded History fixture");
        }

        let loaded = load_history_from_dir(&directory);
        let index = load_history_index_from_dir(&directory);

        assert_eq!(loaded.records.len(), MAX_HISTORY_RECORDS);
        assert_eq!(index.records.len(), MAX_HISTORY_RECORDS);
        assert_eq!(
            loaded.records.first().map(|record| record.id.as_str()),
            Some("record-204")
        );
        assert_eq!(
            loaded.records.last().map(|record| record.id.as_str()),
            Some("record-005")
        );
        assert_eq!(
            index.records.first().map(|record| record.id.as_str()),
            Some("record-204")
        );
        assert_eq!(
            index.records.last().map(|record| record.id.as_str()),
            Some("record-005")
        );
        assert_eq!(loaded.skipped_files, 5);
        assert_eq!(index.skipped_files, 5);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn directory_loader_applies_entry_and_aggregate_byte_budgets_before_parsing() {
        let directory = temp_history_dir("bounded_budget");
        fs::create_dir_all(&directory).expect("create budgeted History directory");
        for index in 0..4 {
            fs::write(
                directory.join(format!("record-{index}.json")),
                format!(
                    r#"{{"version":1,"id":"record-{index}","saved_at":"2026-01-01T00:00:0{index}Z","summary":{{"padding":"{}"}}}}"#,
                    "x".repeat(128)
                ),
            )
            .expect("write budgeted History fixture");
        }
        let limits = HistoryDirectoryLimits {
            max_entries: 3,
            max_main_manifests: 3,
            max_records: 2,
            max_total_bytes: 512,
        };

        let loaded = load_history_from_dir_with_limits(&directory, limits);
        let index = load_history_index_from_dir_with_limits(&directory, limits);

        assert!(loaded.records.len() <= 2);
        assert!(index.records.len() <= 2);
        assert!(loaded.skipped_files >= 2);
        assert!(index.skipped_files >= 2);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn maximum_legal_sidecar_count_does_not_consume_main_manifest_budget() {
        let directory = temp_history_dir("sidecars_do_not_hide_main");
        fs::create_dir_all(&directory).expect("create sidecar budget directory");
        for index in 0..MAX_HISTORY_CHUNKS_PER_RECORD {
            fs::File::create(directory.join(format!("sidecar-{index:04}.nte-history-chunk")))
                .expect("create sidecar budget fixture");
        }
        fs::write(
            directory.join("target.json"),
            r#"{"version":1,"id":"target","saved_at":"2026-01-01T00:00:00Z","summary":{}}"#,
        )
        .expect("write visible main manifest");

        let absolute_entry_budget = MAX_HISTORY_DIRECTORY_ENTRIES;
        assert!(absolute_entry_budget > MAX_HISTORY_CHUNKS_PER_RECORD);
        assert_eq!(
            load_history_index_from_dir(&directory)
                .records
                .first()
                .map(|record| record.id.as_str()),
            Some("target")
        );
        assert_eq!(
            load_history_summaries_from_dir(&directory)
                .records
                .first()
                .map(|record| record.id.as_str()),
            Some("target")
        );
        assert_eq!(
            load_history_from_dir(&directory)
                .records
                .first()
                .map(|record| record.id.as_str()),
            Some("target")
        );
        assert_eq!(
            find_history_index_for_mutation(
                &directory,
                "target",
                HistoryDirectoryLimits::PRODUCTION,
            )
            .expect("find record after sidecars")
            .map(|record| record.id),
            Some("target".to_owned())
        );
        prune_history_dir(&directory, 1).expect("prune scan ignores sidecars for candidate budget");
        assert!(directory.join("target.json").is_file());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn main_manifest_candidate_budget_is_independent_and_mutations_fail_closed() {
        let directory = temp_history_dir("main_manifest_budget");
        fs::create_dir_all(&directory).expect("create main candidate budget directory");
        for index in 0..4 {
            fs::File::create(directory.join(format!("sidecar-{index}.nte-history-chunk")))
                .expect("create sidecar fixture");
        }
        for index in 0..2 {
            fs::write(
                directory.join(format!("record-{index}.json")),
                format!(
                    r#"{{"version":1,"id":"record-{index}","saved_at":"2026-01-01T00:00:0{index}Z","summary":{{}}}}"#
                ),
            )
            .expect("write main candidate fixture");
        }
        let limits = HistoryDirectoryLimits {
            max_entries: 8,
            max_main_manifests: 1,
            max_records: MAX_HISTORY_RECORDS,
            max_total_bytes: MAX_HISTORY_DIRECTORY_BYTES,
        };

        let index = load_history_index_from_dir_with_limits(&directory, limits);
        assert_eq!(index.records.len(), 1);
        assert_eq!(index.skipped_files, 1);
        assert!(matches!(
            find_history_index_for_mutation(&directory, "record-0", limits),
            Err(HistoryDeleteError::ScanBudgetExceeded)
        ));
        assert_eq!(
            prune_history_dir_with_entry_limit(&directory, 1, 8, 1),
            Err(HistoryMaintenanceWarning::RetentionPruneFailed)
        );
        assert_eq!(
            fs::read_dir(&directory)
                .expect("read unchanged candidate fixtures")
                .count(),
            6
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn loads_legacy_version_record() {
        let directory = temp_history_dir("legacy");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("legacy.json"),
            r#"{"version":0,"id":"","saved_at":"2026-01-01T00:00:00Z","summary":{"dps_time_mode":"扣除时停","total_damage":100.0,"abyss":{"detected":true,"active_half":"上行线","first_half":{"half":"Ascending Line"},"second_half":{"half":"下りライン"}}}}"#,
        )
        .unwrap();

        let result = load_history_from_dir(&directory);

        assert_eq!(result.skipped_files, 0);
        assert_eq!(result.records.len(), 1);
        assert_eq!(result.records[0].version, 0);
        assert!(!result.records[0].id.is_empty());
        assert_eq!(
            result.records[0].summary.dps_time_mode,
            DpsTimeBasis::SubtractTimeStop
        );
        let abyss = &result.records[0].summary.abyss;
        assert_eq!(abyss.active_half, Some(AbyssHalf::First));
        assert_eq!(abyss.first_half.as_ref().unwrap().half, AbyssHalf::First);
        assert_eq!(abyss.second_half.as_ref().unwrap().half, AbyssHalf::Second);
        assert!(result.records[0].details.is_none());
        assert!(result.records[0].recorded_at.is_none());
        assert_eq!(
            result.records[0].effective_timestamp(),
            &result.records[0].saved_at
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn history_index_reads_round_metadata_without_materializing_details() {
        let directory = temp_history_dir("index_only");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("indexed.json"),
            r#"{"version":1,"id":"indexed","saved_at":"2026-01-01T00:00:00Z","summary":{"abyss":{"floor":12}},"details":{"global_hits":[{"not":"decoded by the index"}]}}"#,
        )
        .unwrap();

        let result = load_history_index_from_dir(&directory);

        assert_eq!(result.skipped_files, 0);
        assert_eq!(result.records.len(), 1);
        assert_eq!(result.records[0].id, "indexed");
        assert_eq!(result.records[0].abyss_floor, Some(12));
        assert!(result.records[0].has_details);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn importing_the_same_external_id_never_overwrites_local_history() {
        let directory = temp_history_dir("fresh_import_identity");
        let saved_at = DateTime::<Utc>::from_timestamp_millis(1_700_000_000_125).unwrap();
        let mut source = HistoryRecord {
            id: "import-source".to_owned(),
            saved_at,
            summary: CombatSessionSummary {
                total_damage: 100.0,
                ..Default::default()
            },
            details: Some(HistoryCombatDetails {
                global_hits: vec![history_hit(100.0, 1, 10.0)],
                ..Default::default()
            }),
            ..Default::default()
        };

        let first = import_record_json_to_dir(
            &directory,
            &serde_json::to_string(&source).expect("serialize first import"),
        )
        .expect("import first History record");
        source.summary.total_damage = 250.0;
        source.recorded_at = DateTime::<Utc>::from_timestamp_millis(1_700_000_001_125);
        source.details.as_mut().unwrap().global_hits[0].damage = 250.0;
        let second = import_record_json_to_dir(
            &directory,
            &serde_json::to_string(&source).expect("serialize updated import"),
        )
        .expect("import updated History record");

        assert_ne!(first.id, source.id);
        assert_ne!(second.id, source.id);
        assert_ne!(first.id, second.id);
        let entries = fs::read_dir(&directory)
            .expect("read imported History directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        assert_eq!(
            entries
                .iter()
                .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
                .count(),
            2
        );
        let loaded = load_history_from_dir(&directory);
        assert_eq!(loaded.records.len(), 2);
        let mut total_damage = loaded
            .records
            .iter()
            .map(|record| record.summary.total_damage)
            .collect::<Vec<_>>();
        total_damage.sort_by(f64::total_cmp);
        assert_eq!(total_damage, [100.0, 250.0]);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn detailed_record_uses_earliest_hit_time_for_display_file_and_sorting() {
        let directory = temp_history_dir("recorded_at");
        let mut state = CombatState::default();
        state.push_hit(history_hit(1_700_000_005.0, 1, 100.0));
        let mut incoming = history_hit(1_700_000_000.125, 2, 50.0);
        incoming.direction = HitDirection::Incoming;
        state.push_hit(incoming);
        let details = HistoryCombatDetails::from_state(&state).unwrap();

        let record =
            save_summary_with_details_to_dir(&directory, CombatSessionSummary::default(), details)
                .unwrap();
        let expected = DateTime::<Utc>::from_timestamp_millis(1_700_000_000_125).unwrap();

        assert_eq!(record.recorded_at, Some(expected));
        assert_eq!(record.effective_timestamp(), &expected);
        let expected_file_name = format!("{}_{}.json", record.file_timestamp(), record.id);
        let main_files = fs::read_dir(&directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.path().extension().and_then(|value| value.to_str()) == Some("json")
            })
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(main_files, vec![expected_file_name]);
        assert!(
            fs::read_dir(&directory)
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| {
                    entry.path().extension().and_then(|value| value.to_str())
                        == Some("nte-history-chunk")
                })
        );

        let newer_saved_at = DateTime::<Utc>::from_timestamp_millis(1_800_000_000_000).unwrap();
        let later_combat = DateTime::<Utc>::from_timestamp_millis(1_700_000_010_000).unwrap();
        let mut records = vec![
            record,
            HistoryRecord {
                id: "later-combat".to_owned(),
                saved_at: newer_saved_at,
                recorded_at: Some(later_combat),
                ..Default::default()
            },
        ];
        sort_records_newest_first(&mut records);
        assert_eq!(records[0].id, "later-combat");

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn detailed_abyss_record_roundtrips_and_rebuilds_both_halves() {
        let directory = temp_history_dir("details");
        let mut state = CombatState::default();
        state.abyss.floor = Some(12);
        state.abyss.active_half = Some(AbyssHalf::Second);
        state.abyss.first_half_at = Some(10.0);
        state.abyss.second_half_at = Some(20.0);
        state.abyss.first_half.push_hit(history_hit(11.0, 1, 100.0));
        state
            .abyss
            .second_half
            .push_hit(history_hit(21.0, 2, 200.0));
        state
            .time_stop_events
            .push(TimeStopEvent::GamePauseStarted {
                timestamp: 1.0,
                pause_type_mask: 1,
            });
        state.time_stop_events.push(TimeStopEvent::GamePauseEnded {
            timestamp: 2.0,
            pause_type_mask: 1,
        });
        state.apply_time_stop_event(TimeStopEvent::GamePauseStarted {
            timestamp: 12.0,
            pause_type_mask: 1,
        });
        state.apply_time_stop_event(TimeStopEvent::GamePauseEnded {
            timestamp: 13.0,
            pause_type_mask: 1,
        });
        state.rebuild_global_from_abyss();
        let details = HistoryCombatDetails::from_state(&state).unwrap();
        assert_eq!(details.time_stop_events.len(), 2);

        save_summary_with_details_to_dir(&directory, CombatSessionSummary::default(), details)
            .unwrap();
        let records = load_history_from_dir(&directory).records;
        let restored = records[0].details.as_ref().unwrap().to_combat_state();

        assert_eq!(restored.abyss.floor, Some(12));
        assert_eq!(restored.abyss.active_half, Some(AbyssHalf::Second));
        assert_eq!(restored.abyss.first_half.hits.len(), 1);
        assert_eq!(restored.abyss.second_half.hits.len(), 1);
        assert_eq!(restored.hits.len(), 2);
        assert_eq!(restored.total_damage, 300.0);
        assert_eq!(restored.time_stop_events.len(), 2);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn detailed_global_record_roundtrips_without_creating_an_abyss_run() {
        let mut state = CombatState::default();
        state.push_hit(history_hit(11.0, 1, 100.0));
        state.push_hit(history_hit(12.0, 2, 200.0));
        state.apply_time_stop_event(TimeStopEvent::GamePauseStarted {
            timestamp: 11.2,
            pause_type_mask: 1,
        });
        state.apply_time_stop_event(TimeStopEvent::GamePauseEnded {
            timestamp: 11.5,
            pause_type_mask: 1,
        });

        let details = HistoryCombatDetails::from_state(&state).unwrap();
        let restored = details.to_combat_state();

        assert_eq!(details.global_hits.len(), 2);
        assert!(details.first_half_hits.is_empty());
        assert!(details.second_half_hits.is_empty());
        assert!(!restored.abyss.is_active());
        assert_eq!(restored.hits.len(), 2);
        assert_eq!(restored.total_damage, 300.0);
        assert_eq!(restored.time_stop_events.len(), 2);
    }

    #[test]
    fn live_available_clock_is_frozen_as_recorded_in_history() {
        let mut state = CombatState::default();
        state.combat_clock_health = crate::engine::model::CombatClockRuntimeHealth::Available;
        state.push_hit(history_hit(11.0, 1, 100.0));

        let details = HistoryCombatDetails::from_state(&state).expect("History details");
        assert_eq!(
            details.combat_clock_health,
            crate::engine::model::CombatClockRuntimeHealth::Recorded
        );
        assert_eq!(
            details.to_combat_state().combat_clock_health,
            crate::engine::model::CombatClockRuntimeHealth::Recorded
        );
    }

    #[test]
    fn degraded_clock_health_is_preserved_in_history() {
        let mut state = CombatState::default();
        state.combat_clock_health = crate::engine::model::CombatClockRuntimeHealth::DataUnavailable;
        state.push_hit(history_hit(11.0, 1, 100.0));

        let details = HistoryCombatDetails::from_state(&state).expect("History details");
        assert_eq!(
            details.combat_clock_health,
            crate::engine::model::CombatClockRuntimeHealth::DataUnavailable
        );
        assert_eq!(
            details.to_combat_state().combat_clock_health,
            crate::engine::model::CombatClockRuntimeHealth::DataUnavailable
        );
    }

    #[test]
    fn detailed_record_clips_a_pause_that_starts_before_the_first_hit() {
        let mut state = CombatState::default();
        state.apply_time_stop_event(TimeStopEvent::GamePauseStarted {
            timestamp: 10.0,
            pause_type_mask: 1,
        });
        state.push_hit(history_hit(13.0, 1, 100.0));
        state.apply_time_stop_event(TimeStopEvent::GamePauseEnded {
            timestamp: 14.0,
            pause_type_mask: 1,
        });
        state.push_hit(history_hit(20.0, 2, 200.0));

        let details = HistoryCombatDetails::from_state(&state).unwrap();
        let restored = details.to_combat_state();

        assert_eq!(
            details.time_stop_events,
            vec![
                TimeStopEvent::GamePauseStarted {
                    timestamp: 13.0,
                    pause_type_mask: 1,
                },
                TimeStopEvent::GamePauseEnded {
                    timestamp: 14.0,
                    pause_type_mask: 1,
                },
            ]
        );
        assert!((restored.duration_with_time_stop(false) - 7.0).abs() < 1e-9);
        assert!((restored.duration_with_time_stop(true) - 6.0).abs() < 1e-9);
    }

    #[test]
    fn imported_record_preserves_details_and_uses_fresh_local_identity() {
        let source_directory = temp_history_dir("import_source");
        let destination_directory = temp_history_dir("import_destination");
        let mut state = CombatState::default();
        state.push_hit(history_hit(11.0, 1, 100.0));
        let details = HistoryCombatDetails::from_state(&state).unwrap();
        let original = save_summary_with_details_to_dir(
            &source_directory,
            CombatSessionSummary::default(),
            details,
        )
        .unwrap();
        let export_path = source_directory.join("exported.json");
        fs::write(
            &export_path,
            serde_json::to_string_pretty(&original).unwrap(),
        )
        .unwrap();

        let first = import_record_to_dir(&destination_directory, &export_path).unwrap();
        let second = import_record_to_dir(&destination_directory, &export_path).unwrap();

        assert_ne!(first.id, original.id);
        assert_ne!(second.id, original.id);
        assert_ne!(second.id, first.id);
        assert_eq!(first.version, HISTORY_RECORD_VERSION);
        assert_eq!(first.saved_at, original.saved_at);
        assert_eq!(first.recorded_at, original.recorded_at);
        assert_eq!(
            first
                .details
                .as_ref()
                .unwrap()
                .to_combat_state()
                .total_damage,
            100.0
        );
        let loaded = load_history_from_dir(&destination_directory);
        assert_eq!(loaded.skipped_files, 0);
        assert_eq!(loaded.records.len(), 2);
        let _ = fs::remove_dir_all(source_directory);
        let _ = fs::remove_dir_all(destination_directory);
    }

    #[test]
    fn json_text_import_uses_the_same_validation_and_fresh_identity_rules() {
        let directory = temp_history_dir("import_json_text");
        let json = r#"{"version":1,"id":"external-id","saved_at":"2026-01-01T00:00:00Z","summary":{"total_damage":42.0}}"#;

        let imported = import_record_json_to_dir(&directory, json).unwrap();

        assert_ne!(imported.id, "external-id");
        assert_eq!(imported.summary.total_damage, 42.0);
        assert_eq!(load_history_from_dir(&directory).records.len(), 1);
        assert!(import_record_json_to_dir(&directory, "{not json").is_err());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn history_import_rejects_future_versions_and_oversized_files() {
        let source_directory = temp_history_dir("import_invalid_source");
        let destination_directory = temp_history_dir("import_invalid_destination");
        fs::create_dir_all(&source_directory).unwrap();
        let future_path = source_directory.join("future.json");
        fs::write(
            &future_path,
            r#"{"version":2,"id":"future","saved_at":"2026-01-01T00:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(
            import_record_to_dir(&destination_directory, &future_path).unwrap_err(),
            "Unsupported history version 2"
        );

        let oversized_path = source_directory.join("oversized.json");
        fs::File::create(&oversized_path)
            .unwrap()
            .set_len(MAX_HISTORY_IMPORT_BYTES + 1)
            .unwrap();
        assert_eq!(
            import_record_to_dir(&destination_directory, &oversized_path).unwrap_err(),
            "History record exceeds the supported file size"
        );
        let _ = fs::remove_dir_all(source_directory);
        let _ = fs::remove_dir_all(destination_directory);
    }

    #[test]
    fn skips_corrupt_json() {
        let directory = temp_history_dir("corrupt");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("bad.json"), "{not json").unwrap();

        let result = load_history_from_dir(&directory);

        assert_eq!(result.records.len(), 0);
        assert_eq!(result.skipped_files, 1);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn skips_history_with_unsafe_record_id() {
        let directory = temp_history_dir("unsafe_id");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("unsafe.json"),
            r#"{"id":"../outside","saved_at":"2026-01-01T00:00:00Z"}"#,
        )
        .unwrap();

        let result = load_history_from_dir(&directory);

        assert!(result.records.is_empty());
        assert_eq!(result.skipped_files, 1);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn deleted_record_can_be_restored_exactly() {
        let directory = temp_history_dir("restore");
        let record = HistoryRecord {
            id: "restore-me".to_owned(),
            summary: CombatSessionSummary {
                total_damage: 321.0,
                ..Default::default()
            },
            ..Default::default()
        };

        restore_record_to_dir(&directory, &record).unwrap();
        assert!(delete_record_from_dir(&directory, &record.id).unwrap());
        assert!(load_history_from_dir(&directory).records.is_empty());

        restore_record_to_dir(&directory, &record).unwrap();
        let restored = load_history_from_dir(&directory).records;
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id, record.id);
        assert_eq!(restored[0].summary.total_damage, 321.0);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn deleting_record_does_not_match_an_id_suffix() {
        let directory = temp_history_dir("delete_exact_id");
        let selected = HistoryRecord {
            id: "abc".to_owned(),
            ..Default::default()
        };
        let other = HistoryRecord {
            id: "x_abc".to_owned(),
            ..Default::default()
        };
        restore_record_to_dir(&directory, &selected).unwrap();
        restore_record_to_dir(&directory, &other).unwrap();

        assert!(delete_record_from_dir(&directory, &selected.id).unwrap());
        let remaining = load_history_from_dir(&directory).records;
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, other.id);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn delete_fails_closed_before_removal_when_directory_entry_budget_is_exceeded() {
        let directory = temp_history_dir("delete_entry_budget");
        fs::create_dir_all(&directory).expect("create delete budget directory");
        for index in 0..3 {
            fs::write(
                directory.join(format!("record-{index}.json")),
                format!(
                    r#"{{"version":1,"id":"record-{index}","saved_at":"2026-01-01T00:00:0{index}Z","summary":{{}}}}"#
                ),
            )
            .expect("write delete entry fixture");
        }
        let limits = HistoryDirectoryLimits {
            max_entries: 2,
            max_main_manifests: 2,
            max_records: MAX_HISTORY_RECORDS,
            max_total_bytes: MAX_HISTORY_DIRECTORY_BYTES,
        };

        let error = delete_record_from_dir_with_limits(&directory, "record-0", limits)
            .expect_err("incomplete scan must not remove a candidate");

        assert_eq!(error, "History directory exceeds the supported scan budget");
        assert_eq!(fs::read_dir(&directory).expect("read fixtures").count(), 3);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn delete_fails_closed_when_any_json_exceeds_the_aggregate_byte_budget() {
        let directory = temp_history_dir("delete_byte_budget");
        fs::create_dir_all(&directory).expect("create delete byte directory");
        fs::write(
            directory.join("target.json"),
            r#"{"version":1,"id":"target","saved_at":"2026-01-01T00:00:00Z","summary":{}}"#,
        )
        .expect("write delete target");
        fs::File::create(directory.join("oversized.json"))
            .expect("create oversized delete candidate")
            .set_len(2_048)
            .expect("size oversized delete candidate");
        let limits = HistoryDirectoryLimits {
            max_entries: 8,
            max_main_manifests: 8,
            max_records: MAX_HISTORY_RECORDS,
            max_total_bytes: 1_024,
        };

        let error = delete_record_from_dir_with_limits(&directory, "target", limits)
            .expect_err("oversized candidate must abort before deletion");

        assert_eq!(error, "History directory exceeds the supported scan budget");
        assert!(directory.join("target.json").exists());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn prunes_oldest_records() {
        let directory = temp_history_dir("prune");
        fs::create_dir_all(&directory).unwrap();
        for index in 0..3 {
            let mut summary = CombatSessionSummary {
                total_damage: index as f64,
                ..Default::default()
            };
            summary.characters.push(CombatSessionCharacterSummary {
                char_id: index,
                name: format!("角色{index}"),
                damage: index as f64,
                dps: index as f64,
                ..Default::default()
            });
            save_summary_to_dir(&directory, summary).unwrap();
        }

        prune_history_dir(&directory, 2).unwrap();
        let files = fs::read_dir(&directory).unwrap().count();

        assert_eq!(files, 2);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn prune_entry_overflow_returns_typed_warning_without_deleting_unreviewed_files() {
        let directory = temp_history_dir("prune_entry_budget");
        fs::create_dir_all(&directory).expect("create prune budget directory");
        for index in 0..3 {
            fs::write(directory.join(format!("record-{index}.json")), "{}")
                .expect("write prune entry fixture");
        }

        let error = prune_history_dir_with_entry_limit(&directory, 1, 2, 2)
            .expect_err("incomplete prune scan must fail closed");

        assert_eq!(error, HistoryMaintenanceWarning::RetentionPruneFailed);
        assert_eq!(fs::read_dir(&directory).expect("read fixtures").count(), 3);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn committed_write_reports_maintenance_warning_without_losing_record_identity() {
        let directory = temp_history_dir("committed_maintenance_warning");
        let outcome = save_record_to_dir_with_maintenance(
            &directory,
            CombatSessionSummary {
                total_damage: 42.0,
                ..Default::default()
            },
            None,
            |_| Err(HistoryMaintenanceWarning::RetentionPruneFailed),
        )
        .expect("the atomic record write committed before maintenance failed");

        let (record, warning) = match outcome {
            HistorySaveOutcome::CommittedWithMaintenanceWarning { record, warning } => {
                (record, warning)
            }
            HistorySaveOutcome::Committed(_) => {
                panic!("maintenance failure must remain visible in the typed outcome")
            }
        };
        assert_eq!(warning, HistoryMaintenanceWarning::RetentionPruneFailed);
        assert_eq!(
            warning.to_string(),
            "History retention maintenance did not finish."
        );
        let loaded = load_history_from_dir(&directory);
        assert_eq!(loaded.skipped_files, 0);
        assert_eq!(loaded.records.len(), 1);
        assert_eq!(loaded.records[0].id, record.id);
        assert_eq!(loaded.records[0].summary.total_damage, 42.0);
        let compatibility_record =
            compatibility_save_result(Ok(HistorySaveOutcome::CommittedWithMaintenanceWarning {
                record: record.clone(),
                warning,
            }))
            .expect("compatibility callers must not receive a false precommit failure");
        assert_eq!(compatibility_record.id, record.id);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn normal_save_reports_a_clean_committed_outcome() {
        let directory = temp_history_dir("clean_commit_outcome");

        let outcome = save_summary_to_dir_outcome(
            &directory,
            CombatSessionSummary {
                total_damage: 21.0,
                ..Default::default()
            },
        )
        .expect("History record commit");

        assert!(matches!(&outcome, HistorySaveOutcome::Committed(_)));
        assert_eq!(outcome.maintenance_warning(), None);
        assert_eq!(outcome.record().summary.total_damage, 21.0);
        assert_eq!(load_history_from_dir(&directory).records.len(), 1);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn precommit_failure_is_typed_and_creates_no_record() {
        let directory = temp_history_dir("typed_precommit_failure");
        fs::write(&directory, "not a directory").unwrap();

        let error = save_summary_to_dir_outcome(
            &directory,
            CombatSessionSummary {
                total_damage: 7.0,
                ..Default::default()
            },
        )
        .unwrap_err();

        assert_eq!(error, HistorySaveError::PrepareDirectory);
        assert_eq!(
            error.to_string(),
            "History storage directory could not be prepared."
        );
        assert!(fs::metadata(&directory).unwrap().is_file());
        fs::remove_file(directory).unwrap();
    }

    #[test]
    fn invalid_details_fail_before_creating_the_history_directory() {
        let directory = temp_history_dir("invalid_details_precommit");
        let details = HistoryCombatDetails {
            global_hits: vec![history_hit(1.0, 1, 10.0)],
            first_half_hits: vec![history_hit(2.0, 1, 20.0)],
            ..Default::default()
        };

        let error = save_summary_with_details_to_dir_outcome(
            &directory,
            CombatSessionSummary::default(),
            details,
        )
        .unwrap_err();

        assert_eq!(error, HistorySaveError::InvalidDetails);
        assert!(!directory.exists());
    }

    #[test]
    fn runtime_details_are_not_rejected_by_the_removed_legacy_hit_cap() {
        let hit = history_hit(1.0, 1, 10.0);
        let details = HistoryCombatDetails {
            global_hits: vec![hit; MAX_HISTORY_IMPORT_HITS + 1],
            ..Default::default()
        };

        details
            .validate()
            .expect("runtime history must retain every hit beyond the external import cap");
        assert!(
            details.validate_external().is_err(),
            "external inline JSON keeps its independent trust-boundary count budget"
        );
    }

    #[test]
    fn chunked_history_round_trips_without_inline_truncation_and_rejects_external_manifests() {
        let directory = temp_history_dir("chunked_round_trip");
        let hit = history_hit(1.0, 1, 10.0);
        let expected_hits = MAX_HISTORY_HITS_PER_CHUNK + 1;
        let details = HistoryCombatDetails {
            global_hits: vec![hit; expected_hits],
            ..Default::default()
        };

        let saved =
            save_summary_with_details_to_dir(&directory, CombatSessionSummary::default(), details)
                .expect("chunked History save");
        assert_eq!(
            saved
                .details
                .as_ref()
                .map(|details| details.global_hits.len()),
            Some(expected_hits)
        );
        let main_path = fs::read_dir(&directory)
            .expect("read History directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .expect("chunk manifest record");
        let main_text = fs::read_to_string(&main_path).expect("read chunk manifest");
        let main: serde_json::Value = serde_json::from_str(&main_text).expect("parse manifest");
        assert!(main.get("details").is_none() || main["details"].is_null());
        assert!(main.get("details_chunks").is_some());
        let chunk_count = fs::read_dir(&directory)
            .expect("read History chunks")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.path().extension().and_then(|value| value.to_str())
                    == Some("nte-history-chunk")
            })
            .count();
        assert!(chunk_count >= 2);

        let loaded = load_history_record_from_path(&main_path).expect("hydrate History chunks");
        assert_eq!(
            loaded
                .details
                .as_ref()
                .map(|details| details.global_hits.len()),
            Some(expected_hits)
        );
        let import_directory = temp_history_dir("reject_chunk_manifest_import");
        assert!(import_record_json_to_dir(&import_directory, &main_text).is_err());

        assert!(delete_record_from_dir(&directory, &saved.id).unwrap());
        assert_eq!(
            fs::read_dir(&directory)
                .expect("read deleted History directory")
                .count(),
            0,
            "deleting the manifest also removes every owned chunk"
        );
        let _ = fs::remove_dir_all(directory);
        let _ = fs::remove_dir_all(import_directory);
    }

    #[test]
    fn chunked_history_file_export_is_complete_and_self_contained() {
        let directory = temp_history_dir("chunked_file_export");
        let export_path = directory.with_extension("export.json");
        let expected_hits = MAX_HISTORY_HITS_PER_CHUNK + 1;
        let saved = save_summary_with_details_to_dir(
            &directory,
            CombatSessionSummary::default(),
            HistoryCombatDetails {
                global_hits: vec![history_hit(1.0, 1, 10.0); expected_hits],
                ..Default::default()
            },
        )
        .expect("chunked History save");

        export_history_record_by_id_from_dir_to_path(&directory, &saved.id, &export_path)
            .expect("streamed History export");
        let exported: HistoryRecord = serde_json::from_reader(BufReader::new(
            fs::File::open(&export_path).expect("open streamed History export"),
        ))
        .expect("parse streamed History export");
        assert_eq!(
            exported
                .details
                .as_ref()
                .map(|details| details.global_hits.len()),
            Some(expected_hits)
        );
        assert!(exported.details_chunks.is_none());

        let _ = fs::remove_dir_all(directory);
        let _ = fs::remove_file(export_path);
    }

    #[test]
    #[ignore = "large regression: exercises a record beyond the external 500k-hit budget"]
    fn chunked_history_round_trips_more_than_external_hit_budget() {
        let directory = temp_history_dir("chunked_over_external_budget");
        let export_path = directory.with_extension("large-export.json");
        let expected_hits = MAX_HISTORY_IMPORT_HITS + 1;
        let saved = save_summary_with_details_to_dir(
            &directory,
            CombatSessionSummary::default(),
            HistoryCombatDetails {
                global_hits: vec![history_hit(1.0, 1, 10.0); expected_hits],
                ..Default::default()
            },
        )
        .expect("save every hit beyond external import budget");
        let selected =
            load_history_record_by_id_from_dir_for_interactive_selection(&directory, &saved.id)
                .expect("load 500001-hit interactive selection")
                .expect("500001-hit selection exists");
        assert_eq!(
            selected
                .details
                .as_ref()
                .map(|details| details.global_hits.len()),
            Some(expected_hits),
            "interactive selection must retain the 500001-hit regression fixture"
        );
        export_history_record_by_id_from_dir_to_path(&directory, &saved.id, &export_path)
            .expect("stream every hit beyond external import budget");
        let exported: HistoryRecord = serde_json::from_reader(BufReader::new(
            fs::File::open(&export_path).expect("open large streamed History export"),
        ))
        .expect("reload large self-contained History export");
        assert_eq!(
            exported
                .details
                .as_ref()
                .map(|details| details.global_hits.len()),
            Some(expected_hits),
            "file export must retain every hit beyond the external import cap"
        );
        assert!(exported.details_chunks.is_none());
        let main_path = fs::read_dir(&directory)
            .expect("read History directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .expect("chunk manifest record");

        let mut loaded =
            load_history_record_from_path(&main_path).expect("reload every History hit");
        assert_eq!(
            loaded
                .details
                .as_ref()
                .map(|details| details.global_hits.len()),
            Some(expected_hits)
        );
        let rebuilt = loaded
            .details
            .take()
            .expect("loaded History details")
            .into_combat_state();
        assert_eq!(
            rebuilt.hits.len(),
            expected_hits,
            "consuming selection rebuild moves every hit without truncation"
        );
        assert_eq!(
            saved
                .details
                .as_ref()
                .map(|details| details.global_hits.len()),
            Some(expected_hits)
        );
        let _ = fs::remove_dir_all(directory);
        let _ = fs::remove_file(export_path);
    }

    #[test]
    fn corrupted_chunk_export_fails_closed_without_replacing_destination() {
        let directory = temp_history_dir("corrupt_chunk_export");
        let export_path = directory.with_extension("corrupt-export.json");
        let saved = save_summary_with_details_to_dir(
            &directory,
            CombatSessionSummary::default(),
            HistoryCombatDetails {
                global_hits: vec![history_hit(1.0, 1, 10.0); 2],
                ..Default::default()
            },
        )
        .expect("save export corruption fixture");
        let prepared = prepare_history_record_export_from_dir(&directory, &saved.id)
            .expect("prepare export corruption fixture");
        let chunk = fs::read_dir(&directory)
            .expect("read export corruption fixture")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.extension().and_then(|value| value.to_str()) == Some("nte-history-chunk")
            })
            .expect("export corruption chunk");
        let mut bytes = fs::read(&chunk).expect("read export corruption chunk");
        bytes[0] ^= 1;
        fs::write(&chunk, bytes).expect("rewrite same-size corrupt export chunk");
        fs::write(&export_path, b"keep-existing-destination")
            .expect("write existing export destination");

        assert_eq!(
            export_prepared_history_record_to_path(&prepared, &export_path).unwrap_err(),
            HistoryRecordExportError::CorruptRecord
        );
        assert_eq!(
            fs::read(&export_path).expect("read preserved export destination"),
            b"keep-existing-destination"
        );
        let _ = fs::remove_dir_all(directory);
        let _ = fs::remove_file(export_path);
    }

    #[test]
    fn delete_race_is_typed_and_cannot_commit_a_partial_export() {
        let directory = temp_history_dir("delete_during_export");
        let export_path = directory.with_extension("delete-race-export.json");
        let saved = save_summary_with_details_to_dir(
            &directory,
            CombatSessionSummary::default(),
            HistoryCombatDetails {
                global_hits: vec![history_hit(1.0, 1, 10.0); MAX_HISTORY_HITS_PER_CHUNK + 1],
                ..Default::default()
            },
        )
        .expect("save delete race fixture");
        let prepared = prepare_history_record_export_from_dir(&directory, &saved.id)
            .expect("prepare delete race export");
        let tombstone = tombstone_record_from_dir(&directory, &saved.id)
            .expect("move delete race fixture")
            .expect("delete race fixture exists");
        fs::write(&export_path, b"keep-existing-destination")
            .expect("write delete race destination");

        assert_eq!(
            export_prepared_history_record_to_path(&prepared, &export_path).unwrap_err(),
            HistoryRecordExportError::SourceChanged
        );
        assert_eq!(
            fs::read(&export_path).expect("read delete race destination"),
            b"keep-existing-destination"
        );

        restore_tombstoned_record_from_dir(&tombstone).expect("restore delete race fixture");
        export_prepared_history_record_to_path(&prepared, &export_path)
            .expect("the same descriptor remains valid after exact restore");
        let restored_export: HistoryRecord = serde_json::from_reader(BufReader::new(
            fs::File::open(&export_path).expect("open restored delete race export"),
        ))
        .expect("parse restored delete race export");
        assert_eq!(
            restored_export
                .details
                .as_ref()
                .map(|details| details.global_hits.len()),
            Some(MAX_HISTORY_HITS_PER_CHUNK + 1)
        );
        let _ = fs::remove_dir_all(directory);
        let _ = fs::remove_file(export_path);
    }

    #[test]
    fn concurrent_delete_yields_a_complete_export_or_typed_atomic_abort() {
        use std::sync::{Arc, Barrier};

        let directory = temp_history_dir("concurrent_delete_export");
        let export_path = directory.with_extension("concurrent-delete-export.json");
        let expected_hits = MAX_HISTORY_HITS_PER_CHUNK * 2 + 1;
        let saved = save_summary_with_details_to_dir(
            &directory,
            CombatSessionSummary::default(),
            HistoryCombatDetails {
                global_hits: vec![history_hit(1.0, 1, 10.0); expected_hits],
                ..Default::default()
            },
        )
        .expect("save concurrent delete export fixture");
        let prepared = prepare_history_record_export_from_dir(&directory, &saved.id)
            .expect("prepare concurrent delete export");
        fs::write(&export_path, b"keep-existing-destination")
            .expect("write concurrent delete destination");

        let barrier = Arc::new(Barrier::new(2));
        let delete_barrier = Arc::clone(&barrier);
        let delete_directory = directory.clone();
        let delete_record_id = saved.id.clone();
        let delete = std::thread::spawn(move || {
            delete_barrier.wait();
            tombstone_record_from_dir(&delete_directory, &delete_record_id)
                .expect("concurrent tombstone operation")
                .expect("concurrent delete fixture exists")
        });
        barrier.wait();
        let export = export_prepared_history_record_to_path(&prepared, &export_path);
        let tombstone = delete.join().expect("join concurrent tombstone operation");

        match export {
            Ok(()) => {
                let exported: HistoryRecord = serde_json::from_reader(BufReader::new(
                    fs::File::open(&export_path).expect("open concurrent complete export"),
                ))
                .expect("parse concurrent complete export");
                assert_eq!(
                    exported
                        .details
                        .as_ref()
                        .map(|details| details.global_hits.len()),
                    Some(expected_hits)
                );
            }
            Err(HistoryRecordExportError::SourceChanged)
            | Err(HistoryRecordExportError::CorruptRecord) => {
                assert_eq!(
                    fs::read(&export_path).expect("read atomically preserved destination"),
                    b"keep-existing-destination"
                );
            }
            Err(error) => panic!("unexpected concurrent export failure: {error}"),
        }

        restore_tombstoned_record_from_dir(&tombstone)
            .expect("restore concurrent delete export fixture");
        let _ = fs::remove_dir_all(directory);
        let _ = fs::remove_file(export_path);
    }

    #[test]
    fn corrupted_history_chunk_is_rejected_without_partial_details() {
        let directory = temp_history_dir("corrupt_chunk");
        save_summary_with_details_to_dir(
            &directory,
            CombatSessionSummary::default(),
            HistoryCombatDetails {
                global_hits: vec![history_hit(1.0, 1, 10.0); 2],
                ..Default::default()
            },
        )
        .expect("chunked History save");
        let chunk = fs::read_dir(&directory)
            .expect("read History chunks")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.extension().and_then(|value| value.to_str()) == Some("nte-history-chunk")
            })
            .expect("History detail chunk");
        fs::OpenOptions::new()
            .append(true)
            .open(chunk)
            .expect("open History chunk")
            .write_all(b"corrupt")
            .expect("corrupt History chunk");

        let loaded = load_history_from_dir(&directory);
        assert!(loaded.records.is_empty());
        assert_eq!(loaded.skipped_files, 1);
        let summaries = load_history_summaries_from_dir(&directory);
        assert_eq!(summaries.records.len(), 1);
        assert!(summaries.records[0].details.is_some());
        assert_eq!(
            summaries.records[0]
                .details
                .as_ref()
                .map(|details| details.global_hits.len()),
            Some(0),
            "list projection must not open or materialize the corrupted detail chunk"
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn summary_loader_ignores_legacy_inline_hit_payloads() {
        let directory = temp_history_dir("legacy_inline_summary");
        fs::create_dir_all(&directory).expect("create legacy History directory");
        let record = HistoryRecord {
            id: "legacy-inline".to_owned(),
            details: Some(HistoryCombatDetails {
                global_hits: vec![history_hit(1.0, 1, 10.0); 10_000],
                ..Default::default()
            }),
            ..Default::default()
        };
        fs::write(
            directory.join("legacy-inline.json"),
            serde_json::to_vec(&record).expect("serialize legacy inline History"),
        )
        .expect("write legacy inline History");

        let summaries = load_history_summaries_from_dir(&directory);
        assert_eq!(summaries.records.len(), 1);
        assert_eq!(
            summaries.records[0]
                .details
                .as_ref()
                .map(|details| details.global_hits.len()),
            Some(0)
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn detail_byte_preflight_rejects_before_opening_a_missing_chunk() {
        let directory = temp_history_dir("detail_preflight");
        let saved = save_summary_with_details_to_dir(
            &directory,
            CombatSessionSummary::default(),
            HistoryCombatDetails {
                global_hits: vec![history_hit(1.0, 1, 10.0); 2],
                ..Default::default()
            },
        )
        .expect("save chunked History");
        let index = load_history_index_from_dir(&directory);
        let indexed = index.records.first().expect("indexed History");
        assert!(indexed.stored_detail_bytes > 0);
        assert_eq!(indexed.total_detail_hits, Some(2));
        let chunk = fs::read_dir(&directory)
            .expect("read History directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.extension().and_then(|value| value.to_str()) == Some("nte-history-chunk")
            })
            .expect("History detail chunk");
        fs::remove_file(chunk).expect("remove History detail chunk");

        assert!(matches!(
            load_history_record_by_id_from_dir_with_max_detail_bytes(&directory, &saved.id, 0),
            Err(HistoryRecordLoadError::DetailsTooLarge { .. })
        ));
        assert_eq!(
            load_history_record_by_id_from_dir_with_max_detail_bytes(
                &directory,
                &saved.id,
                u64::MAX,
            )
            .unwrap_err(),
            HistoryRecordLoadError::LoadFailed
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn interactive_hit_preflight_rejects_budget_plus_one_before_opening_chunks() {
        let directory = temp_history_dir("interactive_hit_preflight");
        fs::create_dir_all(&directory).expect("create interactive preflight directory");
        let mut remaining_hits = MAX_HISTORY_INTERACTIVE_DETAIL_HITS + 1;
        let mut chunks = Vec::new();
        while remaining_hits > 0 {
            let hit_count = remaining_hits.min(MAX_HISTORY_HITS_PER_CHUNK as u64) as usize;
            chunks.push(HistoryHitChunkRef {
                lane: HistoryHitLane::Global,
                index: chunks.len(),
                hit_count,
                bytes: 1,
                checksum: 0,
            });
            remaining_hits = remaining_hits.saturating_sub(hit_count as u64);
        }
        let record = HistoryRecord {
            id: "interactive-overflow".to_owned(),
            details_chunks: Some(HistoryDetailsChunkManifest {
                version: HISTORY_DETAILS_CHUNK_VERSION,
                metadata: HistoryCombatDetails::default(),
                total_hits: MAX_HISTORY_INTERACTIVE_DETAIL_HITS + 1,
                total_bytes: chunks.len() as u64,
                chunks,
            }),
            ..Default::default()
        };
        fs::write(
            directory.join("interactive-overflow.json"),
            serde_json::to_vec(&record).expect("serialize interactive overflow manifest"),
        )
        .expect("write interactive overflow manifest");
        let mut byte_chunks = (0..32)
            .map(|index| HistoryHitChunkRef {
                lane: HistoryHitLane::Global,
                index,
                hit_count: 1,
                bytes: MAX_HISTORY_CHUNK_BYTES,
                checksum: 0,
            })
            .collect::<Vec<_>>();
        byte_chunks.push(HistoryHitChunkRef {
            lane: HistoryHitLane::Global,
            index: byte_chunks.len(),
            hit_count: 1,
            bytes: 1,
            checksum: 0,
        });
        let byte_record = HistoryRecord {
            id: "interactive-byte-overflow".to_owned(),
            details_chunks: Some(HistoryDetailsChunkManifest {
                version: HISTORY_DETAILS_CHUNK_VERSION,
                metadata: HistoryCombatDetails::default(),
                total_hits: byte_chunks.len() as u64,
                total_bytes: MAX_HISTORY_INTERACTIVE_DETAIL_BYTES + 1,
                chunks: byte_chunks,
            }),
            ..Default::default()
        };
        fs::write(
            directory.join("interactive-byte-overflow.json"),
            serde_json::to_vec(&byte_record).expect("serialize interactive byte overflow manifest"),
        )
        .expect("write interactive byte overflow manifest");

        assert_eq!(
            load_history_record_by_id_from_dir_for_interactive_selection(
                &directory,
                "interactive-overflow",
            )
            .unwrap_err(),
            HistoryRecordLoadError::DetailHitsTooLarge {
                count: MAX_HISTORY_INTERACTIVE_DETAIL_HITS + 1,
                limit: MAX_HISTORY_INTERACTIVE_DETAIL_HITS,
            }
        );
        assert_eq!(
            load_history_record_by_id_from_dir_for_interactive_selection(
                &directory,
                "interactive-byte-overflow",
            )
            .unwrap_err(),
            HistoryRecordLoadError::DetailsTooLarge {
                size: MAX_HISTORY_INTERACTIVE_DETAIL_BYTES + 1,
                limit: MAX_HISTORY_INTERACTIVE_DETAIL_BYTES,
            }
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn external_summary_enforces_global_half_string_finite_and_nested_budgets() {
        let path = Path::new("external-summary.json");
        let mut record = HistoryRecord {
            id: "external-summary".to_owned(),
            ..Default::default()
        };

        record.summary.characters = vec![
            CombatSessionCharacterSummary::default();
            MAX_HISTORY_SUMMARY_CHARACTERS_PER_SCOPE + 1
        ];
        let text = serde_json::to_string(&record).expect("serialize global row overflow");
        assert!(parse_history_record(&text, path).is_err());

        record.summary.characters.clear();
        record.summary.abyss.first_half = Some(CombatSessionAbyssHalfSummary {
            skills: vec![
                CombatSessionSkillSummary::default();
                MAX_HISTORY_SUMMARY_SKILLS_PER_SCOPE + 1
            ],
            ..Default::default()
        });
        let text = serde_json::to_string(&record).expect("serialize half row overflow");
        assert!(parse_history_record(&text, path).is_err());

        record.summary.abyss.first_half = None;
        record.summary.abyss.second_half = Some(CombatSessionAbyssHalfSummary {
            characters: vec![CombatSessionCharacterSummary {
                name: "x".repeat(MAX_HISTORY_SUMMARY_TEXT_BYTES + 1),
                ..Default::default()
            }],
            ..Default::default()
        });
        let text = serde_json::to_string(&record).expect("serialize half string overflow");
        assert!(parse_history_record(&text, path).is_err());

        record.summary.abyss.second_half = None;
        record.summary.duration_seconds = f64::INFINITY;
        assert!(validate_history_summary(&record.summary).is_err());

        let character = CombatSessionCharacterSummary::default();
        let skill = CombatSessionSkillSummary::default();
        let half = CombatSessionAbyssHalfSummary {
            characters: vec![character.clone(); MAX_HISTORY_SUMMARY_CHARACTERS_PER_SCOPE],
            skills: vec![skill.clone(); MAX_HISTORY_SUMMARY_SKILLS_PER_SCOPE],
            ..Default::default()
        };
        let exact_nested = CombatSessionSummary {
            characters: vec![character; MAX_HISTORY_SUMMARY_CHARACTERS_PER_SCOPE],
            skills: vec![skill; MAX_HISTORY_SUMMARY_SKILLS_PER_SCOPE],
            abyss: crate::engine::model::CombatSessionAbyssSummary {
                first_half: Some(half.clone()),
                second_half: Some(half),
                ..Default::default()
            },
            ..Default::default()
        };
        validate_history_summary(&exact_nested).expect("exact nested summary budget");
    }

    #[test]
    fn multi_chunk_tombstone_delete_and_restore_preserves_every_hit() {
        let directory = temp_history_dir("multi_chunk_tombstone");
        let expected_hits = MAX_HISTORY_HITS_PER_CHUNK + 1;
        let saved = save_summary_with_details_to_dir(
            &directory,
            CombatSessionSummary::default(),
            HistoryCombatDetails {
                global_hits: vec![history_hit(1.0, 1, 10.0); expected_hits],
                ..Default::default()
            },
        )
        .expect("save multi-chunk History");

        let tombstone = tombstone_record_from_dir(&directory, &saved.id)
            .expect("tombstone History")
            .expect("History exists");
        assert_eq!(tombstone.token().len(), 37);
        assert!(!tombstone.token().contains(&saved.id));
        assert_eq!(tombstone.record_id(), saved.id);
        assert!(load_history_index_from_dir(&directory).records.is_empty());
        restore_tombstoned_record_from_dir(&tombstone).expect("restore History tombstone");

        let loaded = load_history_record_by_id_from_dir_with_max_detail_bytes(
            &directory,
            &saved.id,
            u64::MAX,
        )
        .expect("load restored History")
        .expect("restored History exists");
        assert_eq!(
            loaded
                .details
                .as_ref()
                .map(|details| details.global_hits.len()),
            Some(expected_hits)
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn tombstone_delete_fails_closed_when_a_sidecar_is_missing_or_corrupt() {
        for corrupt in [false, true] {
            let directory = temp_history_dir(if corrupt {
                "corrupt_tombstone_sidecar"
            } else {
                "missing_tombstone_sidecar"
            });
            let saved = save_summary_with_details_to_dir(
                &directory,
                CombatSessionSummary::default(),
                HistoryCombatDetails {
                    global_hits: vec![history_hit(1.0, 1, 10.0); 2],
                    ..Default::default()
                },
            )
            .expect("save chunked History");
            let chunk = fs::read_dir(&directory)
                .expect("read History directory")
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| {
                    path.extension().and_then(|value| value.to_str()) == Some("nte-history-chunk")
                })
                .expect("History detail chunk");
            if corrupt {
                fs::OpenOptions::new()
                    .append(true)
                    .open(&chunk)
                    .expect("open History detail chunk")
                    .write_all(b"corrupt")
                    .expect("corrupt History detail chunk");
            } else {
                fs::remove_file(&chunk).expect("remove History detail chunk");
            }

            assert_eq!(
                tombstone_record_from_dir(&directory, &saved.id).unwrap_err(),
                HistoryDeleteError::CorruptDetails
            );
            assert_eq!(
                load_history_index_from_dir(&directory).records.len(),
                1,
                "main manifest remains visible when preflight fails"
            );
            let _ = fs::remove_dir_all(directory);
        }
    }

    #[test]
    fn discarded_and_orphaned_tombstones_are_cleaned_without_hydrating_hits() {
        let directory = temp_history_dir("discard_tombstone");
        let first = save_summary_with_details_to_dir(
            &directory,
            CombatSessionSummary::default(),
            HistoryCombatDetails {
                global_hits: vec![history_hit(1.0, 1, 10.0); 2],
                ..Default::default()
            },
        )
        .expect("save first History");
        let first_tombstone = tombstone_record_from_dir(&directory, &first.id)
            .expect("tombstone first History")
            .expect("first History exists");
        discard_tombstoned_record_from_dir(&first_tombstone).expect("discard replaced undo");
        discard_tombstoned_record_from_dir(&first_tombstone)
            .expect("duplicate discard is idempotent");

        let second = save_summary_with_details_to_dir(
            &directory,
            CombatSessionSummary::default(),
            HistoryCombatDetails {
                global_hits: vec![history_hit(2.0, 2, 20.0); 2],
                ..Default::default()
            },
        )
        .expect("save second History");
        let _orphan = tombstone_record_from_dir(&directory, &second.id)
            .expect("tombstone second History")
            .expect("second History exists");
        assert_eq!(
            cleanup_orphaned_history_tombstones(&directory).unwrap(),
            0,
            "startup cleanup must not delete another live owner's undo"
        );
        assert_eq!(
            cleanup_orphaned_history_tombstones_with(
                &directory,
                Utc::now().timestamp_millis(),
                |_| TombstoneOwnerStatus::Dead,
            )
            .unwrap(),
            1,
            "a dead owner's bounded tombstone is reclaimed"
        );
        assert_eq!(cleanup_orphaned_history_tombstones(&directory).unwrap(), 0);

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn tombstone_discard_validates_main_checksum_and_rejects_extra_files() {
        for extra_file in [false, true] {
            let directory = temp_history_dir(if extra_file {
                "tombstone_extra_file"
            } else {
                "tombstone_main_checksum"
            });
            let saved = save_summary_to_dir(&directory, CombatSessionSummary::default())
                .expect("save History tombstone integrity fixture");
            let tombstone = tombstone_record_from_dir(&directory, &saved.id)
                .expect("create History tombstone integrity fixture")
                .expect("History tombstone integrity fixture exists");
            if extra_file {
                fs::write(tombstone.directory.join("unexpected"), b"unexpected")
                    .expect("write unexpected tombstone file");
            } else {
                let mut bytes = fs::read(&tombstone.main.tombstone).expect("read tombstone main");
                let byte = bytes.first_mut().expect("non-empty tombstone main");
                *byte ^= 1;
                fs::write(&tombstone.main.tombstone, bytes)
                    .expect("rewrite same-size tombstone main");
            }

            assert_eq!(
                discard_tombstoned_record_from_dir(&tombstone).unwrap_err(),
                HistoryDeleteError::CleanupFailed
            );
            assert!(tombstone.directory.exists());
            let _ = fs::remove_dir_all(directory);
        }
    }

    #[test]
    fn tombstone_cleanup_accepts_maximum_chunks_plus_main_and_owner() {
        let directory = temp_history_dir("tombstone_file_budget");
        fs::create_dir_all(&directory).expect("create tombstone file budget fixture");
        for index in 0..MAX_HISTORY_TOMBSTONE_FILES_PER_RECORD {
            fs::write(directory.join(format!("file-{index:04}")), b"x")
                .expect("write bounded tombstone file");
        }

        remove_history_tombstone_directory(&directory)
            .expect("maximum chunks plus main and owner remains within budget");
        assert!(!directory.exists());
    }

    #[test]
    fn too_many_time_stop_events_fail_before_chunk_commit() {
        let directory = temp_history_dir("too_many_time_stop_events");
        let details = HistoryCombatDetails {
            time_stop_events: (0..=MAX_HISTORY_IMPORT_TIME_STOP_EVENTS)
                .map(|index| TimeStopEvent::GamePauseStarted {
                    timestamp: index as f64,
                    pause_type_mask: 1,
                })
                .collect(),
            ..Default::default()
        };

        let error = save_summary_with_details_to_dir_outcome(
            &directory,
            CombatSessionSummary::default(),
            details,
        )
        .unwrap_err();
        assert_eq!(error, HistorySaveError::InvalidDetails);
        assert!(!directory.exists());
    }

    #[test]
    fn save_error_messages_are_stable_and_redacted() {
        let cases = [
            (
                HistorySaveError::InvalidSummary,
                "History summary failed validation.",
            ),
            (
                HistorySaveError::InvalidDetails,
                "History details failed validation.",
            ),
            (
                HistorySaveError::PrepareDirectory,
                "History storage directory could not be prepared.",
            ),
            (
                HistorySaveError::Serialize,
                "History record could not be serialized.",
            ),
            (
                HistorySaveError::TooLarge,
                "History record exceeds the supported file size.",
            ),
            (
                HistorySaveError::Commit,
                "History record could not be committed.",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
            assert!(!error.to_string().contains("private-history-path"));
        }
        assert!(!HistorySaveError::InvalidSummary.is_retryable());
        assert!(!HistorySaveError::InvalidDetails.is_retryable());
        assert!(!HistorySaveError::Serialize.is_retryable());
        assert!(!HistorySaveError::TooLarge.is_retryable());
        assert!(HistorySaveError::PrepareDirectory.is_retryable());
        assert!(HistorySaveError::Commit.is_retryable());
    }

    #[test]
    fn abyss_record_labels_and_prediction_teams_use_each_half() {
        let record = HistoryRecord {
            summary: CombatSessionSummary {
                total_dps: 999.0,
                characters: vec![
                    character(1, "上角色", 100.0, 10.0),
                    character(2, "下角色", 200.0, 20.0),
                ],
                abyss: CombatSessionAbyssSummary {
                    detected: true,
                    first_half: Some(CombatSessionAbyssHalfSummary {
                        half: AbyssHalf::First,
                        total_dps: 10.0,
                        characters: vec![character(1, "上角色", 100.0, 10.0)],
                        ..Default::default()
                    }),
                    second_half: Some(CombatSessionAbyssHalfSummary {
                        half: AbyssHalf::Second,
                        total_dps: 20.0,
                        characters: vec![character(2, "下角色", 200.0, 20.0)],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };

        assert_eq!(record.upper_team_dps().unwrap().members[0].id, 1);
        assert_eq!(record.lower_team_dps().unwrap().members[0].id, 2);
    }

    #[test]
    fn compare_records_aggregates_duplicate_skill_rows() {
        let left = HistoryRecord {
            id: "left".to_owned(),
            summary: CombatSessionSummary {
                skills: vec![
                    skill("待映射技能", "未知", 100.0),
                    skill("待映射技能", "未知", 25.0),
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        let right = HistoryRecord {
            id: "right".to_owned(),
            summary: CombatSessionSummary {
                skills: vec![
                    skill("待映射技能", "未知", 10.0),
                    skill("待映射技能", "未知", 5.0),
                ],
                ..Default::default()
            },
            ..Default::default()
        };

        let comparison = compare_records(&left, &right);

        assert_eq!(comparison.skill_deltas.len(), 1);
        let delta = &comparison.skill_deltas[0];
        assert_eq!(delta.left_damage, 125.0);
        assert_eq!(delta.right_damage, 15.0);
        assert_eq!(delta.delta_damage, -110.0);
    }

    #[test]
    fn compare_records_matches_legacy_display_name_to_stable_skill_identity() {
        let left = HistoryRecord {
            id: "legacy".to_owned(),
            summary: CombatSessionSummary {
                skills: vec![skill("Test Ultimate", "Q技能", 100.0)],
                ..Default::default()
            },
            ..Default::default()
        };
        let right = HistoryRecord {
            id: "stable".to_owned(),
            summary: CombatSessionSummary {
                skills: vec![CombatSessionSkillSummary {
                    name: "GA_Test_UltraSkill".to_owned(),
                    category: "Q技能".to_owned(),
                    ability_name: Some("GA_Test_UltraSkill".to_owned()),
                    damage_name: Some("Test Ultimate".to_owned()),
                    damage: 125.0,
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        };

        let comparison = compare_records(&left, &right);

        assert_eq!(comparison.skill_deltas.len(), 1);
        let delta = &comparison.skill_deltas[0];
        assert_eq!(delta.name, "Test Ultimate");
        assert_eq!(delta.ability_name.as_deref(), Some("GA_Test_UltraSkill"));
        assert_eq!(delta.left_damage, 100.0);
        assert_eq!(delta.right_damage, 125.0);
        assert_eq!(delta.delta_damage, 25.0);
    }

    #[test]
    fn compare_records_keeps_distinct_stable_skills_with_shared_display_name() {
        let stable_skill = |ability_name: &str, damage: f64| CombatSessionSkillSummary {
            name: ability_name.to_owned(),
            category: "Q技能".to_owned(),
            ability_name: Some(ability_name.to_owned()),
            damage_name: Some("Shared Display".to_owned()),
            damage,
            ..Default::default()
        };
        let left = HistoryRecord {
            summary: CombatSessionSummary {
                skills: vec![
                    stable_skill("GA_Test_First", 100.0),
                    stable_skill("GA_Test_Second", 200.0),
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        let right = HistoryRecord {
            summary: CombatSessionSummary {
                skills: vec![
                    stable_skill("GA_Test_First", 125.0),
                    stable_skill("GA_Test_Second", 250.0),
                ],
                ..Default::default()
            },
            ..Default::default()
        };

        let comparison = compare_records(&left, &right);

        assert_eq!(comparison.skill_deltas.len(), 2);
        let mut deltas = comparison
            .skill_deltas
            .iter()
            .map(|delta| delta.delta_damage)
            .collect::<Vec<_>>();
        deltas.sort_by(f64::total_cmp);
        assert_eq!(deltas, vec![25.0, 50.0]);
    }

    #[test]
    fn compare_records_preserves_gameplay_effect_identity_without_damage_name() {
        let left = HistoryRecord {
            summary: CombatSessionSummary {
                skills: vec![CombatSessionSkillSummary {
                    name: "GE_Test_Skill_Damage".to_owned(),
                    category: "E技能".to_owned(),
                    gameplay_effect_name: Some("GE_Test_Skill_Damage".to_owned()),
                    damage: 100.0,
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        };

        let comparison = compare_records(&left, &HistoryRecord::default());

        assert_eq!(comparison.skill_deltas.len(), 1);
        let delta = &comparison.skill_deltas[0];
        assert_eq!(delta.name, "GE_Test_Skill_Damage");
        assert_eq!(
            delta.gameplay_effect_name.as_deref(),
            Some("GE_Test_Skill_Damage")
        );
    }

    #[test]
    fn compare_records_keeps_semantic_effects_under_one_ability_distinct() {
        let semantic_skill =
            |effect_name: &str, damage_name: &str, damage: f64| CombatSessionSkillSummary {
                name: damage_name.to_owned(),
                category: "Passive Damage".to_owned(),
                ability_name: Some("GA_Test_Passive".to_owned()),
                gameplay_effect_name: Some(effect_name.to_owned()),
                damage_name: Some(damage_name.to_owned()),
                damage,
                ..Default::default()
            };
        let left = HistoryRecord {
            summary: CombatSessionSummary {
                skills: vec![
                    semantic_skill("GE_Test_Passive_First", "First Component", 100.0),
                    semantic_skill("GE_Test_Passive_Second", "Second Component", 200.0),
                ],
                ..Default::default()
            },
            ..Default::default()
        };

        let comparison = compare_records(&left, &HistoryRecord::default());

        assert_eq!(comparison.skill_deltas.len(), 2);
        assert!(comparison.skill_deltas.iter().any(|row| {
            row.gameplay_effect_name.as_deref() == Some("GE_Test_Passive_First")
                && row.left_damage == 100.0
        }));
        assert!(comparison.skill_deltas.iter().any(|row| {
            row.gameplay_effect_name.as_deref() == Some("GE_Test_Passive_Second")
                && row.left_damage == 200.0
        }));
    }

    #[test]
    fn compare_records_matches_one_ability_across_effect_variants() {
        let skill = |effect_name: Option<&str>, damage: f64| CombatSessionSkillSummary {
            name: "GA_Test_UltraSkill".to_owned(),
            category: "Q技能".to_owned(),
            ability_name: Some("GA_Test_UltraSkill".to_owned()),
            gameplay_effect_name: effect_name.map(str::to_owned),
            damage_name: Some("Test Ultimate".to_owned()),
            damage,
            ..Default::default()
        };
        let left = HistoryRecord {
            summary: CombatSessionSummary {
                skills: vec![skill(Some("GE_Test_UltraSkill1_Damage"), 100.0)],
                ..Default::default()
            },
            ..Default::default()
        };
        let right = HistoryRecord {
            summary: CombatSessionSummary {
                skills: vec![skill(Some("GE_Test_UltraSkill2_Damage"), 125.0)],
                ..Default::default()
            },
            ..Default::default()
        };

        let comparison = compare_records(&left, &right);

        assert_eq!(comparison.skill_deltas.len(), 1);
        assert_eq!(comparison.skill_deltas[0].left_damage, 100.0);
        assert_eq!(comparison.skill_deltas[0].right_damage, 125.0);
        assert_eq!(comparison.skill_deltas[0].delta_damage, 25.0);
    }

    #[test]
    fn compare_records_matches_aggregated_and_single_effect_ability_rows() {
        let skill = |effect_name: Option<&str>, damage: f64| CombatSessionSkillSummary {
            name: "GA_Test_UltraSkill".to_owned(),
            category: "Q技能".to_owned(),
            ability_name: Some("GA_Test_UltraSkill".to_owned()),
            gameplay_effect_name: effect_name.map(str::to_owned),
            damage_name: Some("Test Ultimate".to_owned()),
            damage,
            ..Default::default()
        };
        let left = HistoryRecord {
            summary: CombatSessionSummary {
                skills: vec![skill(None, 175.0)],
                ..Default::default()
            },
            ..Default::default()
        };
        let right = HistoryRecord {
            summary: CombatSessionSummary {
                skills: vec![skill(Some("GE_Test_UltraSkill1_Damage"), 200.0)],
                ..Default::default()
            },
            ..Default::default()
        };

        let comparison = compare_records(&left, &right);

        assert_eq!(comparison.skill_deltas.len(), 1);
        assert_eq!(comparison.skill_deltas[0].delta_damage, 25.0);
    }

    fn temp_history_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "nte_history_test_{}_{}_{}",
            name,
            std::process::id(),
            HISTORY_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn character(char_id: u32, name: &str, damage: f64, dps: f64) -> CombatSessionCharacterSummary {
        CombatSessionCharacterSummary {
            char_id,
            name: name.to_owned(),
            damage,
            dps,
            ..Default::default()
        }
    }

    fn skill(name: &str, category: &str, damage: f64) -> CombatSessionSkillSummary {
        CombatSessionSkillSummary {
            name: name.to_owned(),
            category: category.to_owned(),
            damage,
            ..Default::default()
        }
    }

    fn history_hit(timestamp: f64, char_id: u32, damage: f64) -> Hit {
        Hit {
            timestamp,
            char_id,
            char_name: format!("角色{char_id}"),
            char_known: true,
            damage,
            byte_offset: 0,
            bit_shift: 0,
            char_source: HitCharacterSource::Packet,
            direction: HitDirection::Outgoing,
            target_hp_before: 1_000.0,
            target_hp_after: 1_000.0 - damage,
            target_max_hp: 1_000.0,
            max_hp_reduction: 0.0,
            target_hp_percent: (1_000.0 - damage) / 10.0,
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
}
