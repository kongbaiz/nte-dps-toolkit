# Tauri migration phase 13: Mod enablement and hot-reload status

## Scope

This phase advances only the Mod Studio page from source persistence to Mod
enablement and runtime lifecycle visibility. It does not migrate deployment,
completion, signature help, general script logs, emitted Mod events, or the
full runtime-console toolset.

## Enabled-set transaction

- `plugins/nte-mods.enabled` remains the only enabled-set source.
- `set_mod_studio_document_enabled` is restricted to the stable `console`
  window.
- Rust validates the Mod ID, requires an existing source document, enforces
  the 16-Mod limit, and uses the existing sorted atomic enabled-set write.
- React sends only the desired enable intent and applies the Rust-returned
  workspace snapshot.
- A document with an unsaved source buffer keeps its switch disabled so the
  user does not accidentally enable a stale saved version.
- Overlapping enable writes for the same document are suppressed.

## Runtime lifecycle stream

- The existing native watcher remains responsible for the approximately
  250 ms workspace poll, whole-candidate compilation, atomic program swap,
  previous-version retention, and per-Mod runtime quarantine.
- Tauri polls the bounded native log history away from the main thread and
  publishes only stable lifecycle messages.
- The Channel emits connection snapshots and ordered batches with a decimal
  `generation`, `sequence`, and Windows `timestamp100ns`.
- Reconnects and native sequence resets start a new generation.
- Both Rust and React bound retained lifecycle history to 18 entries and
  deduplicate by sequence.
- Internal pipe and filesystem details stay in Rust; React receives stable
  message keys and arguments.

## UI behavior

- Every Explorer row has an accessible enable switch.
- Pending writes disable only that Mod's switch.
- Enable-set failures stay attached to the affected row.
- The header reports whether the in-game Mod runtime is connected.
- The existing Runtime Console area now shows connection, successful hot
  reload, rejected candidate/rollback, and isolated runtime-fault statuses.
- Copy, clear, filtering, full script logs, and emitted events remain Stage D.

## Automated coverage

- existing-document enable and disable through the shared atomic transaction;
- missing-document rejection without creating an enabled-set file;
- stable enabled-set error serialization without private details;
- desktop-feature Mod log decoding and lifecycle-message filtering;
- runtime reconnect, sequence reset, generation, and native-history
  deduplication;
- typed Channel subscription and cleanup;
- strict runtime-event version, u64 string, ordering, level, and batch bounds;
- enabled workspace projection without discarding the loaded source;
- frontend runtime deduplication and generation reset.

## Manual acceptance gate

1. Open **Console > Mod Studio** with at least two saved Mods and start the
   game with the Mods Plugin installed.
2. Toggle one Mod and confirm `plugins/nte-mods.enabled` changes, the Explorer
   state updates, and the runtime reports a successful hot reload.
3. Make a source buffer dirty and confirm only that document's enable switch
   is disabled until Save or Revert.
4. Edit an enabled `.nte` file externally into an invalid source, trigger
   reload, and confirm the previous working set remains active while the
   rollback status appears.
5. Trigger a runtime fault in one synthetic Mod and confirm that Mod is
   paused, other enabled Mods continue, and a subsequent valid save restores
   it.
6. Close and restart the game, then confirm disconnected, reconnect, and
   generation-reset presentation without duplicate status rows.
7. Check simplified Chinese and Japanese, narrow Console width, and 100%,
   125%, and 150% DPI.
8. Recheck the completed HUD for transparency, dragging, passthrough,
   always-on-top behavior, and smooth updates during hot reload.

Stage D starts after this checklist is accepted.
