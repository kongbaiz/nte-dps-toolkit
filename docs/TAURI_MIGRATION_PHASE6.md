# Tauri migration phase 6: HUD width editing

## Scope

This slice completes the implementation portion of the `hud-spike` Stage D
configuration editor:

- the module panel exposes the existing HUD width as an integer pixel field;
- Enter or blur submits one typed command, while ordinary keystrokes stay
  local to React;
- Escape and invalid drafts restore the latest Rust-authoritative width;
- Rust clamps input with the existing `HUD_WIDTH_MIN` and `HUD_WIDTH_MAX`
  configuration constants;
- the complete `UiConfig` is atomically saved before the presentation revision
  changes;
- after a successful configuration change, Tauri updates the native logical
  width while preserving the current logical height;
- the returned Rust snapshot canonicalizes the displayed input;
- the egui HUD width editor remains available for behavior comparison.

No dependency or contract-version change is needed. The width field already
exists in `HudSnapshot`, and the HTML editor uses the existing `HUD Width`
translation key.

## Stable command boundary

```text
set_hud_width(width: i32)
    -> validate hud-spike window
    -> clamp with the root HUD configuration bounds
    -> clone and sanitize the current HudConfig
    -> skip an unchanged width
    -> atomically save complete UiConfig
    -> publish the new presentation revision
    -> update the native logical width, preserving height
    -> return the authoritative TechnicalSnapshot
```

The frontend validates only that the draft is an integer representable by the
typed Tauri command. It does not duplicate the 280..3840 business range.

## HTML interaction

- The numeric field is the only explicitly selectable text in this editor.
- The input uses a four-pixel step, matching the existing egui control.
- Editing does not resize the window or write configuration on each keystroke.
- Enter and blur share one guarded commit path, avoiding duplicate commands.
- A command in flight disables visibility, ordering, and width controls so one
  configuration transaction finishes before another starts.
- The non-passthrough minimum editor height includes the five module rows and
  width field, including the all-modules-hidden state.

## Manual validation gate

Run:

```powershell
pnpm --dir frontend tauri:dev
```

Verify only `hud-spike`:

1. Enter 280, 380, and 512, then use both Enter and blur; confirm one resize per
   commit and that the current height does not change.
2. Enter a value below 280 and above 3840; confirm the field and native window
   settle at 280 and 3840 respectively.
3. Enter an empty value or decimal, and press Escape after another edit;
   confirm the current authoritative width is restored.
4. Close and relaunch the Tauri HUD; confirm the last saved width is restored.
5. Hide every module, open the editor, and confirm all five rows and the width
   field remain inside the rounded WebView boundary.
6. Resize through the field while the mini timeline is visible; confirm Canvas
   redraws without stretch residue and the HTML overlay stays aligned.
7. Repeat at 100%, 125%, and 150% DPI, including moving between displays with
   different scaling.
