# Tauri migration phase 10: Home passthrough hotkey and HUD position lifecycle

## Scope

This slice closes the remaining Tauri HUD host-integration gaps while
preserving the original HUD interaction:

- the saved `UiConfig.passthrough_hotkey` remains authoritative and defaults
  to `Home`;
- one Windows low-level keyboard hook forwards an unmodified first key press
  into the Tauri adapter without consuming the key;
- pressing the configured key toggles `hud-spike` directly between edit and
  mouse-passthrough states, including while the game is foreground;
- passthrough enablement stays unavailable until the recovery hook reports a
  successful installation;
- a hook failure restores edit mode if passthrough was active;
- no recovery, controller, or auxiliary WebView window is created;
- Tauri does not register the egui debug `F12` shortcut;
- the HUD outer position is stored as physical virtual-desktop coordinates,
  including negative secondary-monitor origins;
- native move events are coalesced for 350 ms before the complete `UiConfig`
  is atomically saved;
- startup keeps a saved position when its title strip is reachable on an
  available monitor, otherwise it centers the HUD in the primary work area;
- the migration-period egui config writer preserves the Tauri-only position
  field that it does not own.

The existing egui HUD remains available as the behavior and visual reference.

## Hotkey boundary

The frontend does not listen for the recovery shortcut because a passthrough
WebView does not own keyboard focus. The platform layer therefore installs the
same class of Windows `WH_KEYBOARD_LL` hook used by the original HUD.

The hook only emits a bounded event:

```text
unmodified configured key first keydown
  -> Rust hotkey channel
  -> Tauri HUD adapter
  -> serialized passthrough transaction
  -> setIgnoreCursorEvents
  -> Acrylic/rounding/height refresh
  -> ordered HUD projection revision
```

Key-repeat messages are coalesced until key-up. Ctrl/Alt/Shift combinations do
not toggle passthrough. Every keyboard event is forwarded to
`CallNextHookEx`, so the hook does not replace game input handling.

The Tauri adapter reads the existing persisted shortcut rather than hardcoding
`Home`; old/default configurations continue to use `Home`, while users who
selected `Insert`, `F8`, or `F9` keep that choice.

Native button and hotkey requests share one passthrough transaction lock. This
prevents a simultaneous button click and key press from interleaving native
cursor state, Acrylic state, content height, and the Rust projection.

## Position lifecycle

`UiConfig.hud_window_position` stores `[x, y]` physical pixels in the Windows
virtual desktop. This avoids applying the startup monitor's DPI scale to a
coordinate saved on another monitor.

The window adapter listens to native `Moved` events, sends only the latest
coordinate to a dedicated worker, and saves after a 350 ms quiet period.
Position changes do not increment the HUD presentation revision because no
React projection changed.

At startup:

1. Tauri applies the Rust-owned HUD size.
2. Available monitor work areas are read.
3. A saved position is retained when at least 64 physical pixels of the window
   and its 32-pixel title strip remain reachable.
4. A position on a disconnected monitor is replaced with the centered primary
   work-area position.
5. Native move tracking is registered only after the restore, so the startup
   adjustment is not treated as user input.

## Manual validation gate

Run:

```powershell
pnpm --dir frontend tauri:dev
```

Verify:

1. Press `Home` in edit mode; the HUD enters passthrough without creating
   another window.
2. Keep the game foreground and press `Home` again; the HUD returns to edit
   mode with pointer input and Acrylic restored.
3. Hold `Home`; only the first keydown toggles. Release and press again to
   perform the next toggle.
4. Confirm `Ctrl+Home`, `Alt+Home`, and `Shift+Home` do not toggle.
5. If the saved shortcut is `Insert`, `F8`, or `F9`, confirm that configured
   key replaces `Home`.
6. Rapidly alternate the header button and configured key; the final native
   state and button state stay aligned.
7. Move the HUD on the primary monitor, wait at least 350 ms, restart, and
   confirm the position is restored.
8. Repeat on a secondary monitor with a negative origin and with
   100%/125%/150% mixed DPI.
9. Save a position on a secondary monitor, disconnect that monitor, restart,
   and confirm the HUD is centered inside the primary work area.
