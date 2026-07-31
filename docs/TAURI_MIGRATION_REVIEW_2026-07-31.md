# Tauri migration review findings (2026-07-31)

## Scope

This review covers the uncommitted Tauri/React migration work through phases
12-17 on `codex/tauri-react-architecture`, including Mod Studio editing and
runtime streaming, Settings integration, capture controls, update checks,
global hotkeys, and the auxiliary Abyss Values window.

The findings below are open follow-up items. They are documented with their
trigger, impact, and acceptance condition so later fixes can stay focused and
independently verifiable.

## Open findings

### REV-01 [P1] Serialize Mod enabled-set mutations

- Affected paths:
  - `src/storage/mod_scripts.rs`
  - `src-tauri/src/commands/mod_studio.rs`
  - `frontend/src/features/mod-studio/use-mod-studio.ts`
- Trigger: toggle two different Mods before the first enabled-set command has
  completed.
- Current behavior: each Rust command independently reads the complete enabled
  set, modifies one ID, and atomically writes its candidate. Two candidates can
  start from the same old set, so the later write can discard the earlier
  mutation. Full workspace responses can also arrive out of order in React.
- Impact: the UI can report a successful toggle while
  `plugins/nte-mods.enabled` silently loses another recent change; hot reload
  then compiles the wrong candidate set.
- Required resolution: serialize enabled-set read/modify/write operations at
  the Rust workspace transaction boundary and expose an ordered generation or
  otherwise guarantee ordered frontend application.
- Acceptance: repeatedly toggle two or more different Mods in rapid succession
  and confirm that the final UI, enabled-set file, and compiled runtime set all
  contain every requested mutation.

### REV-02 [P1] Persist the custom BPF filter

- Affected paths:
  - `src-tauri/src/state.rs`
  - `src/storage/config.rs`
- Trigger: save a custom BPF filter, close the application, and start it again.
- Current behavior: `AppState` always initializes `capture_filter` to `udp`.
  Settings updates replace only the runtime mutex; the value is absent from
  `UiConfig` and its compatibility defaults.
- Impact: the next application session silently returns to the default filter,
  so capture behavior differs from the persisted Settings projection and the
  phase-17 contract.
- Required resolution: add a sanitized, backward-compatible BPF field to
  `UiConfig`, initialize `AppState` from it, and save it in the same serialized
  configuration transaction as the other capture settings.
- Acceptance: save a non-default filter, restart, verify the Settings snapshot,
  and confirm that the restored filter is consumed by the next capture start.

### REV-03 [P1] Compare the installed Mods Plugin version during update checks

- Affected paths:
  - `src-tauri/src/commands/settings.rs`
  - `src/storage/update.rs`
- Trigger: check a signed manifest that contains a Mods Plugin component when
  the same or a newer plugin version is already installed.
- Current behavior: the Settings path calls `installed_app_version`, which sets
  the installed plugin version to `None`. Manifest verification therefore has
  no current plugin version against which to filter the component.
- Impact: an up-to-date or newer local plugin can still be reported as an
  available update, making the Settings status inaccurate whenever the
  manifest includes that component.
- Required resolution: reuse `storage::update::installed_component_versions`
  and map its failure to the stable Settings update error contract.
- Acceptance: exercise manifests with older, equal, and newer app/plugin
  versions and confirm that only strictly newer compatible components appear.

### REV-04 [P2] Recreate or hide the Abyss Values window after close

- Affected paths:
  - `src-tauri/src/commands/settings.rs`
  - `src-tauri/src/windows/abyss_values.rs`
  - `src-tauri/tauri.conf.json`
- Trigger: open Abyss Values, close it with the native title-bar button, and
  select Open Abyss Values again.
- Current behavior: the window is created once from Tauri configuration. The
  open command only looks up the existing label, so a destroyed window produces
  `window_operation_failed` until the application is restarted.
- Impact: an ordinary close action disables the migrated auxiliary page for the
  remainder of the session.
- Required resolution: intercept `CloseRequested` and hide the stable window,
  or recreate it from centralized window configuration when the label is
  absent.
- Acceptance: repeatedly open, native-close, and reopen the window, then close
  Console and confirm that its owned auxiliary window is also closed.

### REV-05 [P2] Deliver startup update-check results to Settings

- Affected paths:
  - `src-tauri/src/commands/settings.rs`
  - `frontend/src/features/settings/use-settings.ts`
- Trigger: open Console while the startup automatic update check is still in
  progress.
- Current behavior: Settings requests one initial snapshot, while the automatic
  check changes Rust state later without a Settings channel, event, or bounded
  polling path.
- Impact: the mounted page can remain at `checking` or another stale status
  until a manual refresh or unrelated Settings mutation returns a new snapshot.
- Required resolution: publish low-frequency update-status events through a
  stable Settings subscription, or coordinate initial snapshot loading with the
  automatic check lifecycle.
- Acceptance: test direct networking, a Windows proxy, slow responses, and
  transport failure; verify that checking, current, available, and error states
  all settle in the mounted page without manual refresh.

## Validation snapshot

Passed during the review:

- `git diff --check`;
- frontend lint, typecheck, 77 tests, and production build;
- root and Tauri formatting, check, and tests;
- GUI and CLI feature checks and tests;
- CLI dependency isolation scan with zero forbidden GUI/WebView dependencies;
- Mods Plugin Release x64 clean build with zero warnings and zero errors.

Repository-wide gates still reporting existing baseline findings:

- frontend `format:check` reports 15 files, including the generated lockfile;
  the HEAD lockfile reports the same formatting condition;
- GUI/CLI Clippy reports `nonminimal_bool` in unchanged lines of
  `src/app/mod.rs` and `src/engine/parser.rs`.

## Manual acceptance focus

After the findings are addressed, repeat the phase-specific checks at narrow
and wide Console widths, 100%/125%/150% DPI, all three languages, light and dark
themes, reduced motion, loading/empty/error states, and the documented HUD
passthrough and multi-window lifecycle paths.
