# Tauri migration phase 7: content-owned HUD height

## Scope

This correction aligns `hud-spike` resizing with the existing egui HUD:

- users can continue resizing the native window horizontally;
- the Rust-projected module content owns the logical height;
- startup locks native minimum and maximum height to the same content height;
- module visibility and passthrough transitions update both the authoritative
  size and the height constraints;
- width edits restore the same authoritative height instead of preserving an
  externally introduced temporary height;
- the existing egui HUD and its horizontal-only grips remain unchanged.

No dependency, DTO, command, or frontend permission change is needed.

## Native constraint boundary

For each Rust-authoritative content height, Tauri applies:

```text
minimum logical size = (HUD_WIDTH_MIN, content height)
maximum logical size = (HUD_WIDTH_MAX, content height)
```

When the height grows, the maximum constraint is expanded before raising the
minimum. When it shrinks, the minimum is lowered before reducing the maximum.
This avoids a transient invalid range and lets module or passthrough changes
move directly to the new content height.

The window remains `resizable: true` because width still supports native edge
resizing. Equal minimum and maximum heights block vertical and corner-driven
height changes at the native window boundary.

## Manual validation gate

Run:

```powershell
pnpm --dir frontend tauri:dev
```

Verify only `hud-spike`:

1. Drag the left and right native edges; width changes while height remains
   stable.
2. Drag the top, bottom, and all four corners; content height does not move,
   flicker, or rebound.
3. Hide and restore each HUD module; the Rust-driven height changes once and
   the new height is immediately locked.
4. Toggle passthrough off and on; display and editor heights remain
   content-owned after each transition.
5. Enter several widths through the module panel; each width applies without
   changing the current authoritative height.
6. Repeat at 100%, 125%, and 150% DPI and while moving between displays with
   different scaling.
