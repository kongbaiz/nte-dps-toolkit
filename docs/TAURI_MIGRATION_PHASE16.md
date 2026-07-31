# Tauri migration phase 16: complete Settings surface

## Scope

This phase completes the React layout for the remaining Console Settings
content in one visual pass. It mirrors the migration-period egui Settings
inventory while keeping the accepted Rust-backed HUD controls unchanged.

The following sections are present:

- Interface;
- Software Update;
- Parse Settings;
- Hotkeys;
- HUD;
- Layout Profiles;
- Team Data;
- Capture Files;
- Abyss Values.

## Layout

- Settings use the same category boundaries as the egui page: related controls
  share one card and individual fields are compact divided rows rather than
  nested cards.
- At 1100 px and above, Interface/Update/Parse/Hotkeys occupy the first column
  while HUD/Layout/Team/Capture/Abyss occupy the second.
- Below the breakpoint, both columns stack into one scrollable sequence without
  squeezing localized labels or controls.
- The scroll surface keeps a one-pixel top/left inset so the shadcn card ring is
  not clipped by the overflow boundary.
- The duplicate HUD field-visibility panel remains absent. Visibility and order
  stay combined in the draggable module rows.

## Integration boundary

- HUD width, always-on-top, module visibility, module order, and editor opening
  continue to use the versioned Settings contract from phase 15.
- The other sections are complete UI surfaces with local draft interaction only.
  They do not invoke filesystem, update, capture, hotkey, import/export, window,
  or configuration commands in this phase.
- The existing Rust/egui implementation remains authoritative for those
  operations until each stable DTO and command is migrated.
- No new dependency, capability, command, event, or duplicated Rust business rule
  is introduced.

## Manual acceptance gate

1. Open **Console > Settings** and confirm all nine category cards are present in
   the order listed above.
2. Verify Interface contains language, theme preset, accent, density, reduced
   motion, floating notification, and horizontal-offset controls.
3. Verify Software Update contains version, automatic check/download, idle
   status, proxy guidance, and the update-check action.
4. Verify Parse Settings contains BPF, capture NIC, damage calibration, reaction
   separation, idle round, DPS time, and passthrough-hotkey controls.
5. Verify Hotkeys contains the global toggle, three action bindings, disable
   actions, and the command-palette hint.
6. Verify Layout Profiles, Team Data, Capture Files, and Abyss Values contain the
   same actions and supporting text as the egui page.
7. Confirm the HUD card still loads persisted Rust state and all accepted phase
   15 HUD actions continue to work.
8. Check the current simplified-Chinese projection at 820 px and 1440 px
   Console widths, then repeat at 100%, 125%, and 150% DPI. Repeat the same
   matrix for Japanese when the language command is connected.
9. Confirm the page scrolls as one surface, card columns align when wide, and no
   field label or action is clipped when stacked.

Backend integration for the UI-only sections starts after this visual inventory
is accepted.
