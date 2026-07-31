# Tauri migration phase 17: Settings integration

## Scope

This phase connects the complete phase-16 Settings surface to Rust-owned state.
The versioned Settings contract now projects interface, update, capture,
hotkey, HUD, team-data, capture-file, and window-operation state. React keeps
only transient form text and shortcut-recording state.

## Connected behavior

- Interface language, theme preset, accent, density, reduced motion, floating
  notification preference, and notification offset persist through `UiConfig`.
  Language and presentation changes are applied immediately and shared with
  auxiliary WebViews through a bounded local presentation projection.
- Automatic-update preferences persist. Startup and manual checks use the
  existing signed official manifest, WinHTTP proxy fallback, and Rust
  verification code; transport and signature details stay in Rust logs.
- BPF, capture NIC, damage calibration, reaction separation, idle-round
  settings, DPS time, and HUD passthrough hotkey persist. BPF and NIC choices
  are consumed on the next capture start.
- The shared low-level Windows hotkey hook now handles the three configurable
  modifier-plus-function-key actions without registering a standalone F12
  shortcut. Capture toggle, session reset, and HUD visibility are dispatched
  by the Tauri window/runtime adapter.
- Layout profiles apply the migrated window subset: combat opens the minimal
  compact passthrough HUD; review and research return to Console and hide the
  HUD. Pages that have not yet migrated remain outside this phase.
- Team DPS JSON is size/version/value validated in the frontend-neutral Rust
  core. Import state is runtime-only; export combines the current Rust combat
  state with imported abyss fallbacks.
- Capture-file statistics and cleanup use the existing Rust capture-log
  manager. Locked active files are retained and reported in Rust logs.
- Abyss values open in a stable, capability-scoped auxiliary window. A
  versioned Tauri contract loads the same authoritative `AbyssMonsterDataset`
  used by the egui reference page, including season/floor metadata, route and
  wave assignment, static HP, all stat fields, star thresholds, recommended
  elements, and imported prediction teams. React renders the original
  season/floor explorer, line prediction panels, monster cards, portraits, and
  stat detail grid without parsing the raw abyss tables.
- Settings and abyss mutation failures use fixed dismissible alerts. They stay
  outside the card/grid flow and therefore do not resize either Settings
  column or the abyss explorer.

## Contract and trust boundary

- `SETTINGS_CONTRACT_VERSION = 2`;
  `ABYSS_VALUES_CONTRACT_VERSION = 1`.
- Every command is scoped to the stable Console window.
- Enum-like values, shortcut bindings, BPF text, finite numbers, imported JSON,
  and file sizes are validated at their respective Rust/typed-client boundary.
- 64-bit capture byte totals cross the WebView boundary as decimal strings.
- Configuration writes are serialized and saved before the in-memory
  projection is replaced.
- Capture, combat, updater verification, window, filesystem, and hotkey work
  remain in Rust; React does not reproduce those rules.

## Manual acceptance gate

1. Open **Console > Settings**, change every Interface field, restart, and
   confirm persistence. Repeat language changes for English, Japanese, and
   Simplified Chinese; verify the Console title and auxiliary abyss window.
2. Trigger **Check for updates** with direct networking, a Windows proxy, and
   no network. Confirm checking/up-to-date/available/error status and that no
   transport details appear in the WebView.
3. Refresh the NIC list, select automatic and a manual NIC, edit BPF, then start
   a new capture. Confirm the new selection/filter applies only to that start.
4. Record, disable, and re-enable each global shortcut. Verify capture toggle,
   session reset, and HUD visibility; confirm Alt+F4 and unmodified function
   keys are rejected and no standalone F12 behavior appears.
5. Apply combat/review/research profiles. Confirm combat requires the hotkey
   hook before entering passthrough, and the other profiles restore Console
   control without leaving an unreachable HUD.
6. Import valid/invalid/oversized team JSON and export with live data, imported
   data, and no data.
7. Create capture logs, refresh totals, clear while stopped, then clear during
   an active capture and confirm the locked active file remains.
8. Open and reopen **Abyss Values**. Compare the 7-season/82-floor/843-enemy
   summary, floor and wave counts, monster HP and raw stats with the egui
   reference. Expand seasons, switch floors, search by name/pool/ID, select
   monster cards, import/clear/swap prediction teams, and use star-time chips.
   Then close Console and confirm the HUD and abyss windows also close.
9. Trigger a Settings mutation error and an abyss import error. Confirm the
   alert overlays the page without moving, shrinking, or clipping any card,
   and that its close button dismisses it.
10. Repeat the page at 820 px and 1440 px widths, 100%/125%/150% DPI, light/dark
   state, all three theme presets, and reduced motion.
