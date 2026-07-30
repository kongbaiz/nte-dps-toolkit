# Tauri migration phase 8: native HUD width persistence

## Scope

This slice closes the remaining parity gap between the native Tauri resize
boundary and the existing egui HUD:

- left/right native edge resizing remains immediate;
- physical window pixels are converted to logical HUD width with the active DPI
  scale factor;
- continuous resize events are coalesced for 350 ms;
- only the final logical width enters the existing atomic HUD configuration
  transaction;
- a successful save publishes a new presentation revision through the existing
  ordered Channel;
- startup restores the last successfully saved logical width;
- the Rust-owned content height and equal native height constraints are
  unchanged.

No dependency, DTO, command, capability, or frontend permission change is
needed.

## Resize and persistence boundary

The native window event callback performs only:

```text
physical width / active scale factor
  -> rounded logical width
  -> HUD_WIDTH_MIN..HUD_WIDTH_MAX
  -> lightweight in-process queue
```

A named worker waits until resize events have been quiet for 350 ms, matching
the existing egui configuration debounce. Intermediate widths are replaced by
the newest value. The final value uses `AppState::set_hud_width`, so native
resizing and the HTML width field share sanitization, atomic file replacement,
rollback, and save-before-publish behavior.

Programmatic width changes also emit a native resize event. Their final value
already matches the authoritative configuration, so the state transaction
returns a no-op and skips a second file write.

On a DPI transition, `ScaleFactorChanged` updates the conversion factor before
queuing the new logical width. On window destruction, the worker flushes the
last pending width before exiting.

## Manual validation gate

Run:

```powershell
pnpm --dir frontend tauri:dev
```

Verify only `hud-spike`:

1. Drag the left or right native edge continuously; resizing remains immediate
   and the Rust-owned height stays fixed.
2. Stop dragging, keep the module panel open, and confirm the width field
   updates to the final native width after the next ordered snapshot.
3. Resize rapidly across several widths, wait at least 350 ms, close and reopen
   the Tauri HUD, and confirm only the final width is restored.
4. Submit another value through the HTML width field, resize natively again,
   and confirm both inputs converge on one authoritative value.
5. Repeat at 100%, 125%, and 150% DPI and across displays with different scale
   factors; restart restores the logical width without a scale-factor offset.
6. Resize while capture snapshots and the Canvas timeline are updating; data
   refresh and line redraw remain smooth.
