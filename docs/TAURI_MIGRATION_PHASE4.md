# Tauri migration phase 4: HUD module visibility editor

## Scope

This slice continues only the `hud-spike` window. It adds one bounded part of
the local HUD editor:

- an edit-mode HTML module panel lists modules in Rust-provided order;
- each switch sends one typed visibility intent;
- Rust applies the existing `HudConfig::set_module_visible` semantics;
- the complete UI config is atomically saved before the in-memory projection
  changes;
- a successful change publishes a new snapshot generation and synchronizes the
  native content height;
- the egui HUD editor remains intact for behavior comparison.

Module drag-and-drop ordering and width editing remain outside this slice. No
dependency is added.

## Command and persistence boundary

```text
set_hud_module_visibility(module, visible)
    -> validate window label
    -> validate stable module id
    -> clone current HudConfig candidate
    -> apply existing Rust visibility semantics
    -> atomically save complete UiConfig
    -> publish presentation revision
    -> synchronize native HUD height
    -> return ordered TechnicalSnapshot
```

Accepted module identifiers are:

```text
title | summary | status | characters | timeline
```

Malformed identifiers return a stable command error. Save details stay in the
Rust log; the frontend receives only the stable display key. A failed save
keeps both the prior file and prior in-memory projection.

The frontend does not optimistically change module state. The switch changes
only after the returned Rust snapshot passes the existing TypeScript boundary.
Unknown future module values remain ignored by the current editor.

## Editor layout

- The module button exists only in non-passthrough edit mode.
- The panel is an HTML overlay inside the existing blurred HUD surface.
- Labels reuse the shared translation keys for Title, Summary, Status,
  Character Ranking, and Curve.
- Switches reflect configuration visibility, not whether a data-dependent
  module happens to have content in the current combat.
- The native height follows configured content while retaining enough edit-mode
  room to reopen the module panel when most modules are hidden.
- The edit-mode minimum height keeps all five module rows inside the WebView
  even when every module is disabled.
- Mouse passthrough and always-on-top use one icon button each. The active
  button is highlighted and pressing it sends the inverse state; no adjacent
  switch is rendered.
- Passthrough mode removes the panel and uses the display-only content height.

## Manual validation gate

Run:

```powershell
pnpm --dir frontend tauri:dev
```

Verify only `hud-spike`:

1. In edit mode, open the HUD modules button and confirm all five modules follow
   the saved Rust order.
2. Disable Summary, Character Ranking, and Curve one at a time; each module
   disappears only after the command completes.
3. Re-enable every hidden module from the same panel.
4. Close and relaunch the Tauri HUD; the selected visibility remains saved.
5. Verify the window height follows large visibility changes without changing
   the current width.
6. Hide every module, reopen the panel, and confirm all five rows remain fully
   visible and can be restored.
7. Enter passthrough mode; the editor panel disappears and the remaining HUD
   content stays transparent and mouse-transparent.
8. Confirm the passthrough and always-on-top icons act as highlighted toggle
   buttons without adjacent switches.
9. Compare Summary and Status hide/restore semantics with the egui HUD editor:
   restoring Summary enables team DPS, and restoring Status enables the edit
   state readout.
10. Repeat at narrow width and 100%, 125%, and 150% DPI.
