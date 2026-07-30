# Tauri migration phase 9: always-on-top preference transaction

## Scope

This slice aligns the Tauri HUD pin button with the existing application
preference lifecycle:

- the button still applies the native always-on-top state immediately;
- `UiConfig.always_on_top` is saved through the existing atomic configuration
  writer;
- the Rust projection and ordered Channel revision change only after a
  successful save;
- a failed save keeps the previous Rust projection and restores the previous
  native window state;
- concurrent pin intents are serialized across the native operation and config
  transaction;
- repeated requests for the current value skip the file write and Channel
  update;
- startup continues to apply the saved preference before the HUD is shown.

No dependency, DTO, command, capability, frontend permission, or translation
change is needed.

## Window and configuration boundary

The typed command follows this sequence:

```text
validate hud-spike window
  -> acquire always-on-top transaction lock
  -> remember previous Rust preference
  -> apply requested native window level
  -> atomically save complete UiConfig
  -> update Rust projection and presentation revision
```

If the native operation fails, the configuration remains unchanged. If the
configuration save fails, the state transaction rolls its candidate value back
and the window adapter reapplies the previous native level. The existing stable
`hud_config_save_failed` error is returned to React without exposing the
internal file path or error chain.

The transaction lock covers both the native operation and configuration save,
so concurrent typed intents cannot commit in one order while applying the
native window level in another.

Mouse passthrough remains transient and starts in edit mode, matching the
existing recovery boundary. This slice only persists the user preference that
already exists in `UiConfig`.

## Manual validation gate

Run:

```powershell
pnpm --dir frontend tauri:dev
```

Verify only `hud-spike`:

1. Turn always-on-top off and confirm the button highlight and native window
   level both update.
2. Close and reopen the Tauri HUD; both remain off.
3. Turn always-on-top on, restart again, and confirm both remain on.
4. Repeat the two states while a borderless game window is foreground.
5. Click the button rapidly and confirm the returned snapshot always matches
   the final native state without highlight flicker.
6. Toggle passthrough independently and confirm it still starts in edit mode
   after restart while the pin preference is retained.
