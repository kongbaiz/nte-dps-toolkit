# Tauri Mod Studio migration plan

## Goal

Migrate the existing egui Mod Studio into the stable Tauri `console` window
without moving script validation, workspace persistence, enable-set
transactions, plugin IPC, or hot-reload rules into React.

The software-side workspace remains available independently from game
installation detection. Deployment checks the game path only when the user
performs a deployment operation.

## Stages

### Stage A: read-only workspace foundation

- [x] stable `console` window and scoped capability;
- [x] frontend-neutral Rust workspace index and on-demand source detail;
- [x] versioned Tauri DTO and typed TypeScript client;
- [x] Explorer, selection, source preview, loading, empty, and error states;
- [x] existing Console/Mod Studio visual hierarchy and local light theme;
- [x] read-only line numbers and dependency-free presentation highlighting;
- [x] source-only text selection;
- [x] manual list, selection, refresh, DPI, and HUD regression validation.

### Stage B: source editor

- [x] initially use a dependency-free editable overlay for the basic save
      slice, then retire it after acceptance feedback when Monaco becomes the
      production source surface;
- [x] keep multi-file selection and unsaved-buffer lifecycle in the Mod
      feature;
- [x] send explicit save intents through the typed Tauri client;
- [x] reuse the existing `validate_mod_source` and atomic file write
      transaction;
- [x] return stable diagnostics with original source line numbers;
- [x] preserve the previous saved file when validation or persistence fails;
- [x] support Save, Revert, `Ctrl+S`, Tab indentation, cursor position, and
      per-document dirty state;
- [x] manual editing, save failure, selection, DPI, and HUD regression
      validation.

### Stage C: enable set and hot reload

- [x] reuse `nte-mods.enabled` as the single enabled-set source;
- [x] enable/disable through Rust validation and atomic persistence;
- [x] keep the native runtime's complete candidate-set success requirement
      before runtime swap;
- [x] report runtime connection, hot-reload, rollback, and isolated-fault
      results through generation-aware, ordered, bounded Channel batches;
- [x] keep a single Mod fault quarantined in the native runtime while the
      remaining Mods continue;
- [x] disable enable-set changes while that document has an unsaved buffer;
- [x] manual enable/disable, hot-reload rollback, game reconnect, fault
      isolation, DPI, and HUD regression validation.

### Stage D: completion, signatures, and console

- [x] generate editor completion and signature data from one stable host-API
      schema rather than copying the egui completion table into TypeScript;
- [x] use Monaco's VS Code editor core for native word selection, selection
      painting, occurrence highlights, C++ tokenization, hover, completion,
      signature help, diagnostics, widget placement, and light/dark themes;
- [x] complement Monaco's basic C++ grammar with bounded semantic tokens for
      NTE macros, variables, parameters, namespaces, types, functions, and SDK
      properties;
- [x] keep automatic completion closed, expose explicit `Ctrl+Space`, and show
      schema/document documentation only when hovering a known symbol;
- [x] stream bounded runtime logs and Mod events with sequence handling;
- [x] provide copy, clear, filtering, disconnected, and reconnect states;
- [x] retain the previous working Mod set across compiler or runtime failures;
- [x] manual completion, signature, raw log, event, copy, clear, filtering,
      reconnect, DPI, and HUD regression validation.

Each stage changes only the Mod Studio page, runs focused and full validation,
and stops for user acceptance before the next stage.
