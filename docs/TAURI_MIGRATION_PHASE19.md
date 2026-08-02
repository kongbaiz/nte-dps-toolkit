# Tauri migration phase 19: History page

## Scope

This phase migrates only the Console History page. Rust remains the source of
truth for archive preparation, record validation, bounded persistence, fresh
import identifiers, comparison, prediction-team projection, and deletion.
React renders a bounded versioned DTO and sends typed intents.

## Connected behavior

- The frontend-neutral core prepares one summary and optional bounded detail
  archive from the same combat state. The egui and Tauri entry points reuse it.
- Tauri commands load, save, import JSON text, export JSON, delete/restore,
  compare, and apply upper/lower prediction teams on blocking workers. A
  transaction lock serializes access to the on-disk history directory.
- The live capture service archives the previous round before an Abyss restart
  or floor boundary, and the Tauri History runtime applies the existing idle
  timeout rule. Manual reset uses the same save-before-clear transaction; a
  failed write keeps the current combat round intact.
- Imported records retain the existing 128 MiB cap, version/detail validation,
  atomic write, 200-record pruning, and fresh local identifier behavior.
- The v2 History DTO contains a monotonic revision, localized character/skill
  display names, party labels, prediction availability, truncation counts, and
  at most eight character/skill rows. Raw hits, local paths, undo records, and
  internal errors do not cross the WebView boundary.
- A typed History Channel emits the initial snapshot and later revisions. Its
  subscription is released while React Activity hides the page and restored
  when History becomes visible again.
- Delete returns an opaque single-use token. React exposes a five-second Undo
  action while Rust retains the record inside the trusted process.
- The page preserves the selected record during refresh and mutations instead
  of replacing the mounted page with a loading view. This avoids whole-page
  flashes when a record or prediction action completes.
- Record navigation, detail columns, abyss halves, and row groups respond to
  the available page/container width instead of assuming a maximized window.
  Character rows reuse the bundled character catalog and avatar resources;
  unknown or synthetic rows keep a readable initial fallback.
- Empty, initial loading, stale-with-error, malformed import, skipped corrupt
  file, non-abyss, two-half abyss, comparison-warning, and narrow layouts have
  explicit UI states.
- The empty state can start live capture through the existing typed technical
  command. Capture-JSON replay remains visibly disabled until the later detail
  window owns replay rendering; history-record JSON import stays available.
- Half cards include total damage; skill rows include share and follow-up state;
  bounded projections disclose the number of additional rows in the full
  record rather than silently hiding them.
- JSON export is returned by Rust and downloaded through a temporary browser
  Blob. This avoids a new native-dialog dependency in this phase.

## Automated coverage

- Rust covers empty/current archive preparation, detail preservation, JSON-text
  import validation/fresh IDs, bounded projection, and existing comparison and
  persistence rules.
- TypeScript covers contract versioning, decimal counters, bounded DTO parsing,
  typed History subscription cleanup, typed comparison command routing,
  selection preservation, adjacent comparison, and Console route enablement.

## Manual acceptance gate

1. Open Console > History with no records. Confirm the empty state, Save,
   Import, and Reload controls in English, Japanese, and Simplified Chinese.
2. Produce live combat data and select Save This Summary. Confirm a new newest
   record appears without the page flashing or losing its selection.
3. Import a valid exported JSON twice. Confirm both imports receive distinct
   local records. Try malformed, future-version, oversized, and corrupt-detail
   files and confirm the mounted page remains usable with a localized error.
4. Select records with regular combat and abyss halves. Compare timestamps,
   time basis, reaction accounting, DPS/damage/duration, parse-quality counters,
   character rows, skill rows, and upper/lower half summaries against egui.
5. Compare adjacent and manually selected records. Confirm Rust-provided total,
   character, and skill deltas and the time-basis/reaction warning.
6. Export and re-import a record, delete a record after confirmation, restore it
   within five seconds, and apply upper/lower prediction teams. Confirm Settings
   reports the imported line and Undo expires cleanly.
7. Enable automatic round creation, deal damage, and wait for the configured
   idle threshold. Confirm the record appears without opening/reloading History.
   Repeat with manual reset and an Abyss restart/floor transition; confirm each
   previous round is saved once and the active round continues.
8. Repeat at 820 px, 1000 px, 1280 px, and 1440 px, 100%/125%/150% DPI, with
   long localized names. Confirm the record list becomes a horizontal strip in
   narrow windows, abyss halves and row groups stack before they become cramped,
   all scroll areas remain reachable, and known characters retain their avatars.

Detailed hit replay from `hasDetails` remains owned by the later main/detail
window migration; this History-page phase intentionally exposes only bounded
summaries and does not add a second combat renderer.
