# NTE DPS TOOL frontend

Vite + React + TypeScript + Tailwind CSS + shadcn/ui frontend for the Tauri
desktop application.

This is the production desktop UI. Rust remains the authoritative owner of
domain and window state, while React consumes typed commands and ordered,
throttled snapshot Channels through the Tauri adapter.

## Commands

```powershell
pnpm install --frozen-lockfile
pnpm format:check
pnpm lint
pnpm typecheck
pnpm test
pnpm build
pnpm tauri:dev
```

Run these commands from this directory. See `../AGENTS.md` for the current
architecture boundaries and manual Windows validation checklist.

## Browser UI preview

Run `pnpm dev --host 127.0.0.1 --port 5178 --strictPort` and open
`http://127.0.0.1:5178/ui-preview.html?theme=light` in the Codex browser.
The preview uses the production Console components with synthetic data from
`dev/fixtures.ts`. Its banner identifies the simulation. Native actions such as
capture, saving files and controlling plugins are unavailable. This does not
validate Tauri IPC, native windows or live game capture.

Query options: `theme=light|dark`, `language=en`, `density=compact|comfortable`,
`preset=high-contrast|tactical`, `motion=reduced` and timeline `state=empty|error`.
Theme links reload the preview; settings changes are local to this browser.
The HTML entry is excluded from production build inputs. Check the preview with
`pnpm exec tsc -p dev/tsconfig.json` and `pnpm exec oxlint src dev`.

Layout references: [shadcn dashboard](https://ui.shadcn.com/blocks),
[Sidebar Dashboard Inset](https://blocks.so/sidebar/sidebar-02), and
[Stats with Card Layout](https://blocks.so/stats/stats-03). These inform the
inset workspace and metric hierarchy; the implementation uses existing project
components without adding dependencies.

Plugin controls preview: `ui-preview.html?theme=light&plugin=unavailable` or
`plugin=connected` (synthetic values) and `plugin=packet`. Select Mod Workshop
in the real console navigation. Native actions remain unavailable. Use
`view=data-source&plugin=packet` for the homepage source selector and
`loading=loader` for the managed Loader layout. Mode/loading changes affect only
preview fixtures; Loader startup and file operations remain unavailable.

## Character account snapshots

Character Data is now a read-only User plugin view, not a resource editor.
Choose Plugin mode on the home page and load `NTE_PluginUser.dll`. **Check
snapshot** reads the latest published snapshot; **Refresh from game** requests
one bounded native refresh and follows its status. The page shows capture time,
partial/outdated states, 16-character search pages and one selected loadout.

The Rust core projects schema 1 from Toolkit commands 302/303; refresh uses 301.
It checks process and snapshot identities and never parses the 8 KiB diagnostic
preview as account data. Character levels, arc, saved/base/effective skill
levels, active awakening slots, cassettes and drive blocks come from the plugin.
Missing values remain unknown. Equipment modifier values retain their raw precision. The character build sheet
uses exact attribute IDs in the equipment catalog for display names and percentage
units, with up to two decimal places and no trailing zeros; unknown IDs keep raw
units. Base modifier IDs are not stat values. The game's native account-character calculator now supplies HP maximum, attack,
defense, crit, crit damage, general damage bonus and charge efficiency for all
owned characters, including off-team characters with the character menu closed.
Values reflect current account configuration at refresh, not temporary combat
buffs. Unverified bindings or unreadable values remain unknown; static resources
and historical hits never substitute for live values. Awakening numbers come
from the game's definition order, independent of selected slot order. Sidebar
portraits use the exact character ID in the bundled avatar catalog. Skill labels use plugin semantic categories, not
ability IDs. New plugins advertise `characterRefreshSupported`: the page then
uses command 301 argument 1 to omit the unrelated full inventory scan.

The browser preview includes synthetic character data. Use
`ui-preview.html?theme=light&characters=partial` or `characters=unavailable` to
exercise those states, then select Character Data. This does not validate a live
game or native IPC.

Character panel artwork is exported by `python scripts/export_character_panel_art.py`
from the CN equipment-plan and arc item tables. Only referenced portraits and arc
icons are copied; cassettes and drive blocks reuse existing equipment icons.
No account stats, recommended loadouts, or scores are imported from those tables.

### Empty Curtain / Console equipment

Plugin mode refreshes the User provider's bounded `EQUIP` inventory with permanent
`CARD` slot references. Packet/replay inventory remains the source in replay or
packet views. Filters, character grids, recommended loadouts, lock/discard and
equip/unequip/move operations, calculator export and loadout JSON import/export
reuse the existing core rules and UI. A refresh button no longer requires starting
packet capture. Current CN layout data also includes character plans 1036, 1042,
1057 and 1072.

The existing native capture pipe forwards equipment RPCs to User. During capture,
a capacity-one relay shares that connection and continues draining hit/clock
notifications; it does not open a competing capture connection. Commands freeze
process/provider/account identity, check fresh inventory before dispatch, and
confirm the requested state from readback. Only stale reads may be retried; a
mutation is never automatically resent. Missing data stays an error, not empty
inventory or a successful operation. The wire/TS snapshot contract remains v2.

Equipment refresh has independent busy state and retains the visible inventory;
Channel snapshots do not stop its spinner. While this page is subscribed, a
cancelable backend worker renews the User equipment-change watch every 500 ms.
The native watch expires after two seconds without renewal. Unchanged revisions
do not rescan inventory. Lock/discard notifications carry bounded UID deltas and
refresh only those flags; structural/unknown changes require a full snapshot.
Lock/discard commands also inspect and confirm only their target item. Other
loadout operations inspect the selected identities before dispatch and retain
full post-operation readback. Game-rejected requests remain unconfirmed rather
than being reported as successful or automatically replayed.

Dispatch and confirmation errors remain distinct. If an acknowledged equipment
request races inventory notifications, only snapshot reads are retried. An
unavailable readback becomes a bounded pending confirmation with the original
process/provider/account and requested operation frozen; matching refreshed
state confirms it. Source changes or expiry leave an explicit unconfirmed result,
never a replayed mutation. Transient background snapshot changes do not raise an
operation-failed toast, and recovery clears only the background error it owns.

## Managed plugin components

The homepage shows an explicit current-mode label and checked selection. Mod
Workshop has no manual path picker or host logging-level editor. The Mod Market
owns `mods/game`, `mods/tools` and `mods/driver` next to the executable; see
`../plugins/README.md` for catalog schema 6 and conservative proxy deployment.
Host and generic plugin management only checks status, loads and unloads; plugin
feature controls retain their existing confirmations and runtime capability checks.
