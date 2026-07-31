# Tauri migration phase 12: Mod Studio source editor

## Scope

This phase advances only the Mod Studio page from read-only source preview to
source editing. The existing egui Mod Studio remains the behavior reference,
and enable-set changes, hot reload, completion, signature help, and runtime
console streaming remain in later stages.

No frontend editor package was added. The page reuses the Stage A source
highlighter underneath a controlled `textarea`, keeping the Vite bundle and
runtime model unchanged while providing the editing behaviors needed for this
stage.

## Rust and Tauri boundary

- `save_mod_studio_document` is available only to the stable `console`
  window.
- The command receives an explicit Mod ID and source body, then performs file
  work on Tauri's blocking task pool.
- Rust verifies that the selected document exists and routes the source
  through the existing `validate_mod_source` plus atomic write transaction.
- Validation and persistence failures leave the previous file in place.
- Contract version 2 adds stable save error codes and an optional
  one-based `diagnosticLine`; internal paths and compiler detail stay in Rust
  logs.

## React behavior

- Each opened document owns a source buffer and a last-saved source.
- Switching documents preserves dirty buffers.
- Save and Revert operate only on the selected document.
- A save response updates the saved baseline without discarding edits typed
  while the request was running.
- A synchronous per-document save guard prevents overlapping writes.
- `Ctrl+S` saves, Tab inserts four spaces, and the status bar reports cursor
  position, UTF-8 byte use, dirty/saving/saved/error state, and the diagnostic
  line.
- The editable overlay retains Stage A line numbers and NTE C++ presentation
  highlighting without moving validation into TypeScript.

## Automated validation

The stage covers:

- valid atomic save and reload;
- rejected source preserving the previous file and original line number;
- stable Tauri error serialization without internal detail;
- command names and payloads in the typed client;
- contract version, source-size bounds, and diagnostic parsing;
- dirty-buffer merge, revert, in-flight edit preservation, and summary
  projection;
- cursor position and Tab indentation helpers.

## Manual acceptance gate

1. Open **Console > Mod Studio** with at least two `.nte` files.
2. Edit the selected source and confirm the dirty marker, status text, byte
   count, line numbers, cursor position, selection, scrolling, and syntax
   colors remain aligned.
3. Press `Ctrl+S`, then reopen the document and confirm the saved text is
   loaded.
4. Edit a document, switch to another file, switch back, and confirm the
   unsaved buffer remains.
5. Use **Revert changes** and confirm only the selected buffer returns to its
   last saved source.
6. Introduce an invalid source line, save, and confirm the localized error and
   highlighted line are shown; reopen the file and confirm the previous saved
   source remains.
7. Repeat with an oversized source and confirm the byte limit error is shown.
8. Check simplified Chinese and Japanese, a narrow Console window, 100%,
   125%, and 150% DPI.
9. Confirm the completed HUD still drags in edit mode and retains transparent,
   passthrough, and always-on-top behavior.

Stage C starts only after this checklist is accepted.
