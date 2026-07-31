# Tauri migration phase 15: Console Settings HUD controls

## Scope

This phase starts the first post-Mod-Studio Console tab and migrates the HUD
configuration slice of **Settings**. It keeps `HudConfig` in Rust as the single
source of truth and adds navigation between the accepted Mod Studio and the
new Settings page.

Interface language/theme, updates, capture parsing, global hotkeys, layout
profiles, team import/export, capture-file maintenance, and abyss tables remain
in the migration-period egui Settings page for later Settings slices.

## HUD startup and ownership

- The Tauri HUD window is still configured and initialized, but starts with
  `visible: false`.
- The Console is the migration-period owner of the undecorated HUD window.
  Closing the Console closes the HUD as well, so no invisible or frameless
  window keeps the process alive.
- **Open HUD Editor** shows, restores, focuses, and explicitly returns the HUD
  to editing mode.
- No global F12 registration is introduced.

## Stable Settings contract

- `SETTINGS_CONTRACT_VERSION = 1` returns only adapter version, persisted
  always-on-top state, validated HUD width bounds, and the frontend-neutral
  `HudConfigSnapshot`.
- Commands are scoped to the stable `console` window.
- Option and preset identifiers are allowlisted once at the Tauri boundary.
- Width values reuse Rust's existing `HUD_WIDTH_MIN` and `HUD_WIDTH_MAX`.
- Every mutation saves the existing `UiConfig` transaction before returning a
  fresh snapshot.
- Native HUD width and height are synchronized after a successful persisted
  change.

## React Settings behavior

- The stable `console` route now hosts a reusable sidebar and switches only
  between the migrated **Settings** and **Mod Studio** pages.
- Both migrated pages remain mounted while the other is hidden, preserving Mod
  selection, dirty buffers, runtime history, and Settings state across tab
  switches.
- The Settings page has loading, command-error, and mutation-error states.
- HUD width, always-on-top state, grouped module visibility, and module order all
  use the typed Settings client.
- HUD controls use one compact card with internal window and module-order
  sections instead of splitting one settings category across multiple cards.
- Visibility is controlled only from the draggable module-order rows; the page
  does not repeat those switches in a second field-selection section.
- Module order supports pointer drag-and-drop plus accessible up/down buttons.
- A single pending mutation disables overlapping writes.
- Other Console rows remain visibly disabled until their individual migration
  stages.

## Automated coverage

- Settings DTO version, width bounds, canonical module order, and malformed
  boundary input;
- typed command names and camel-case arguments;
- HUD option and preset allowlists;
- persisted option and preset transactions;
- grouped module visibility and insert-before/insert-after arrow semantics;
- Console route and migrated-page registry.

## Manual acceptance gate

1. Start the Tauri application and confirm only **Console** appears; the HUD is
   absent from the desktop and taskbar.
2. Close Console immediately and confirm the process exits without leaving a
   background HUD window.
3. Open **Console > Settings** and verify loading resolves to one compact HUD
   card with window and module-order sections.
4. Toggle visibility for all five module rows and confirm the HUD editor reflects
   the same persisted state without a second field-selection section.
5. Reorder all five modules by drag-and-drop and by keyboard-accessible arrow
   buttons. Confirm hidden modules retain their canonical positions and can be
   restored.
6. Enter widths below/inside/above the displayed range and confirm Rust clamps
   and returns the authoritative value.
7. Toggle always-on-top, select **Open HUD Editor**, and confirm the hidden HUD
   appears focused in editing mode with mouse input enabled.
8. With the HUD open, close Console and confirm both windows close and the
   process exits.
9. Exercise a read-only config directory and confirm the existing HUD state is
   retained while a localized save error appears.
10. Check simplified Chinese and Japanese, an 820 px Console, and 100%, 125%,
    and 150% DPI.
11. Recheck Mod Studio selection, hover, completion, signature help, runtime
    console, and dirty buffers after switching repeatedly between the two
    pages.

The next Settings slice starts after this checklist is accepted.
