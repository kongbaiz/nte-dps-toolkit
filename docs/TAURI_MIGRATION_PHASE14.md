# Tauri migration phase 14: Mod intelligence and runtime console

## Scope

This phase completes the current Mod Studio migration slice with schema-driven
completion, signature help, full bounded runtime logs, emitted Mod events, and
local console tools. It does not migrate another Console tab, deployment, Mod
creation/deletion, or the final Console window replacement.

## Single Mod SDK schema

- `src/core/mod_sdk.rs` is the frontend-neutral fact source for 88 stable NTE
  C++ declarations, snippets, properties, and functions.
- The migration-period egui editor now consumes this Rust schema instead of
  owning a second static completion table.
- `get_mod_studio_sdk_schema` exposes a separately versioned, bounded DTO only
  to the stable `console` window.
- React validates the contract once, then combines schema symbols with
  functions, parameters, and variables declared in the active document.
- Monaco completion is explicit through `Ctrl+Space`; automatic quick
  suggestions and trigger-character suggestions stay disabled so normal typing
  does not open a popup.
- Signature help tracks nested calls, quoted strings, and the active parameter.
- The source surface now uses the actual Monaco editor core shared by VS Code,
  with its native word selection, selection painting, occurrence highlights,
  C++ tokenization, scrollbar, completion, signature, marker, and hover
  widgets. This removes the transparent textarea and syntax-mirror layers that
  could cover selected text.
- Monaco's basic C++ tokenizer is complemented by a bounded document semantic
  token projection. It colors `NTE_*` macros, document variables, parameters,
  namespaces, C++/NTE types, local and SDK functions, and SDK properties while
  excluding comments, strings, and preprocessor directive bodies.
- Hover documentation is generated from the same Rust SDK schema and active
  document symbols. Monaco shows it only while the pointer rests on a known
  variable or function, using the VS Code default 300 ms hover delay.
- A status-bar action switches Monaco between its built-in `vs` and `vs-dark`
  themes. Monaco owns caret/hover/widget edge avoidance rather than a second
  application overlay implementation.

## Editor dependency

- `monaco-editor` 0.56.0 is maintained by Microsoft and published under MIT.
- The ESM editor is bundled locally with a local editor worker; no CDN or
  remote runtime asset is used.
- The editor is lazy-loaded only when a ready Mod source document is shown.
  Imports are limited to C++ tokenization and the VS Code editing
  contributions used by this page, avoiding unused JSON/CSS/HTML/TypeScript
  language workers.
- Monaco replaces the dependency-free textarea/mirror prototype because the
  latter could not match native VS Code selection, occurrence, hover, and
  widget behavior without reimplementing an editor core.

## Runtime stream

- Rust reads the existing native log and Mod-event histories away from the UI
  thread.
- Lifecycle messages carry a stable translation key; user script logs preserve
  their validated UTF-8 text.
- Log and event native sequences remain separate. The Tauri adapter merges new
  entries by timestamp and assigns one ordered stream sequence per generation.
- Native sequence resets and reconnects start a new generation. Rust batches
  remain bounded to 18 logs plus 18 events; React retains at most 256 entries.
- Mod-event values cross the JavaScript boundary as decimal strings.

## Runtime Console behavior

- The console displays timestamps, source level, Mod ID, raw logs, translated
  lifecycle messages, event names, and decimal/hexadecimal event values.
- Filters cover all entries, logs, events, info, warnings, and errors.
- Copy exports only the currently visible entries.
- Clear creates a local sequence watermark, so native history does not
  immediately repopulate while later entries continue to arrive.
- Disconnect, reconnect, filter-empty, and no-entry states remain distinct.

## Automated coverage

- schema bounds, uniqueness, version, stable symbols, and Rust-to-Tauri
  projection;
- egui completion and signature behavior against the shared schema;
- frontend schema validation and typed client command;
- prefix replacement, local declarations, nested signatures, and suppression
  inside comments and strings;
- semantic classification for macros, variables, parameters, namespaces,
  types, functions, and SDK properties, including false-positive suppression
  inside comments, strings, and directives;
- raw log preservation and lifecycle-key projection;
- log/event timestamp merge, stream sequence, native-history deduplication, and
  generation reset;
- runtime DTO bounds, event values, filters, clear watermark, and formatting.

## Manual acceptance gate

1. Open **Console > Mod Studio**, double-click `player_controller`, and confirm
   exactly that word is selected with readable foreground text. Drag across
   several lines and confirm the selected text is never covered.
2. Place the caret in, or select, `player_controller`; confirm matching
   occurrences use Monaco's VS Code-style occurrence highlighting.
3. Type normally and move the pointer through blank space; confirm no
   completion or documentation popup opens. Rest the pointer on a known local
   variable, local function, or `nte::` SDK function; confirm documentation
   appears after the hover delay and disappears when leaving the symbol.
4. Type `nte::memory::read_`, press `Ctrl+Space`, then use arrow keys, Enter,
   Tab, and Escape. Confirm insertion replaces only the active token and the
   Monaco suggestion widget keeps its selected row visible.
5. Switch the status-bar editor theme between Light and Dark; compare syntax,
   active line, selection, occurrence highlights, hover, suggestion/signature
   widgets, markers, scrollbars, and status bar with VS Code. Confirm
   `NTE_SCRIPT`, `NTE_MOD`, `NTE_REQUIRES`, `NTE_ROUTE_IPC`, declarations,
   parameters, `std`/`nte` namespaces, types, functions, and SDK properties
   retain distinct semantic colors.
6. Declare a local function and variables, then confirm hover and explicit
   completion update without reopening the document.
7. Type nested `nte::ipc::emit(...)` calls and confirm signature help follows
   the active parameter while ignoring commas inside strings and nested calls.
8. Run a synthetic Mod that emits `nte::log::info(...)` and custom IPC events;
   confirm raw logs and decimal/hex event values appear once in timestamp order.
9. Exercise every console filter, Copy, Clear, game disconnect/reconnect, and a
   native sequence reset.
10. Check simplified Chinese and Japanese, narrow Console width, source
    selection, and 100%, 125%, and 150% DPI.
11. Recheck the completed HUD for transparency, dragging, passthrough,
    always-on-top behavior, and smooth updates under active Mod logging.

The next migration slice starts after this checklist is accepted.
