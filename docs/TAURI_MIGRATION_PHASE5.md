# Tauri migration phase 5: HUD module ordering

## Scope

This slice continues only the `hud-spike` module editor:

- every module row has a dedicated pointer-capture drag handle;
- dropping on the upper or lower half of a target maps to insert-before or
  insert-after;
- Up and Down arrow keys on a focused handle provide the same adjacent move;
- React sends one typed move intent and waits for the returned Rust snapshot;
- Rust reuses `HudConfig::move_module` and atomically saves the complete
  `UiConfig`;
- the egui HUD editor remains available for behavior comparison.

HUD width editing remains outside this slice. No dependency is added.

## Stable command boundary

```text
move_hud_module(dragged, target, insertAfter)
    -> validate hud-spike window
    -> validate both stable module identifiers
    -> clone the current sanitized HudConfig
    -> apply HudConfig::move_module
    -> skip a no-op order
    -> atomically save complete UiConfig
    -> publish the new presentation revision
    -> return the ordered TechnicalSnapshot
```

Accepted identifiers remain:

```text
title | summary | status | characters | timeline
```

The command transports module identifiers and one insertion direction only.
The frontend does not send a complete replacement array and does not update the
authoritative order optimistically. Visibility and ordering use the same
save-before-publish transaction.

## HTML interaction

- Dragging starts only from the grip and captures that pointer, so the
  visibility switch remains independent.
- The interaction does not use native HTML `draggable` or `DataTransfer`; this
  avoids WebView2's system no-drop cursor.
- A cyan line marks the exact insertion boundary.
- The dragged row is dimmed locally; other rows stay stable until Rust returns.
- All switches and grips are disabled while a visibility or move command is in
  flight.
- A focused grip accepts Up and Down arrow keys.
- Unknown future module identifiers remain filtered by the existing typed
  projection boundary.

## Manual validation gate

Run:

```powershell
pnpm --dir frontend tauri:dev
```

Verify only `hud-spike`:

1. Open HUD modules and drag Title to the lower half of Character Ranking; it
   should appear immediately after Character Ranking after the command returns.
2. Drag Curve to the upper half of Summary; it should appear immediately before
   Summary.
3. Confirm the cyan insertion line switches between the target row's upper and
   lower edge at its midpoint.
4. Focus a grip and press Up or Down; confirm one adjacent move per key press.
5. Drag hidden and visible modules; ordering must remain independent of
   visibility.
6. Close and relaunch the Tauri HUD; confirm the panel order and rendered HUD
   order are preserved.
7. Confirm dragging a grip does not move the native window, toggle visibility,
   or start another command.
8. Repeat at narrow width and 100%, 125%, and 150% DPI.
