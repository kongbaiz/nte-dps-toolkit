# Tauri Mod Studio migration plan

## Goal

Migrate the existing egui Mod Studio into the stable Tauri `console` window
without moving script validation, workspace persistence, enable-set
transactions, plugin IPC, or hot-reload rules into React.

The software-side workspace remains available independently from game
installation detection. Deployment checks the game path only when the user
performs a deployment operation.

## Stages

### Stage A: read-only workspace foundation (current)

- [x] stable `console` window and scoped capability;
- [x] frontend-neutral Rust workspace index and on-demand source detail;
- [x] versioned Tauri DTO and typed TypeScript client;
- [x] Explorer, selection, source preview, loading, empty, and error states;
- [x] existing Console/Mod Studio visual hierarchy and local light theme;
- [x] read-only line numbers and dependency-free presentation highlighting;
- [x] source-only text selection;
- [ ] manual list, selection, refresh, DPI, and HUD regression validation.

### Stage B: source editor

- choose the editor dependency after reviewing package size, worker/runtime
  integration, maintenance, license, and Vite build output;
- keep multi-file selection and unsaved-buffer lifecycle in the Mod feature;
- send explicit save intents to Rust;
- reuse the existing `validate_mod_source` and atomic file write transaction;
- return stable diagnostics with original source line numbers;
- preserve the previous saved file when validation or persistence fails.

### Stage C: enable set and hot reload

- reuse `nte-mods.enabled` as the single enabled-set source;
- enable/disable through Rust validation and atomic persistence;
- keep the complete candidate-set success requirement before runtime swap;
- report runtime initialization and hot-reload results through ordered,
  bounded messages;
- isolate a single Mod fault from the rest of the application.

### Stage D: completion, signatures, and console

- generate editor completion and signature data from one stable host-API
  schema rather than copying the egui completion table into TypeScript;
- stream bounded runtime logs and Mod events with sequence handling;
- provide copy, clear, filtering, disconnected, and reconnect states;
- retain the previous working Mod set across compiler or runtime failures.

Each stage changes only the Mod Studio page, runs focused and full validation,
and stops for user acceptance before the next stage.
