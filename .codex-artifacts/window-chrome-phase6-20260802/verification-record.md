# Window chrome / context-menu verification record

Date: 2026-08-02 (Asia/Shanghai)  
Workspace: `D:\NTE_DPS_TOOL`  
Scope: shared titlebar always-on-top icon, removal of the main toolbar text pin, capture-phase suppression of the WebView browser context menu, and the Tauri capability required by the affected windows.

## Inputs

- Preserved baseline: `baseline-manifest.json` plus `originals/`
- Modified manifest: `modified-manifest.json`
- Configured windows: `main-dps`, `hud-spike`, `notification-island`, `console`, `abyss-values`, `character-details`, `team-details`
- Shared titlebar consumers: main DPS, Console, Abyss Values, character details, team details
- Existing special windows: HUD has its own pin icon; notification island is configured permanently always-on-top.

## Baseline behavior probe

Command:

```powershell
python .codex-artifacts/window-chrome-phase6-20260802/verify-behavior.py baseline
```

Literal output:

```text
baseline_titlebar_pin=False
baseline_toolbar_text_pin=True
baseline_context_menu_suppression=False
```

Exit status: `0`

## Modified behavior probe

Command:

```powershell
python .codex-artifacts/window-chrome-phase6-20260802/verify-behavior.py modified
```

Literal output:

```text
configured_windows=abyss-values,character-details,console,hud-spike,main-dps,notification-island,team-details
titlebar_pin_windows=abyss-values,character-details,console,main-dps,team-details
hud_pin_control=verified
notification_island_always_on_top=true
native_context_menu_capture_suppression=verified
```

Exit status: `0`

## Focused frontend tests

Command:

```powershell
pnpm --dir frontend test -- src/lib/tauri/desktop-window-client.test.ts src/lib/browser-context-menu.test.ts
```

Literal output:

```text
Test Files  2 passed (2)
Tests       2 passed (2)
```

Exit status: `0`

The context-menu test verifies both results: the native default is prevented and the application-level context-menu handler still executes. The desktop-window client test verifies `isAlwaysOnTop()` and `setAlwaysOnTop(true)` forwarding.

## Frontend static validation

Command:

```powershell
pnpm --dir frontend typecheck
```

Literal output:

```text
$ tsc -b
```

Exit status: `0`

Command:

```powershell
pnpm --dir frontend lint
```

Literal output:

```text
$ oxlint
```

Exit status: `0`

Command:

```powershell
pnpm --dir frontend exec prettier --check src/components/nte/desktop-titlebar.tsx src/lib/tauri/desktop-window-client.ts src/lib/tauri/desktop-window-client.test.ts src/features/main-dps/main-dps-page.tsx src/lib/browser-context-menu.ts src/lib/browser-context-menu.test.ts src/main.tsx
```

Literal output:

```text
Checking formatting...
All matched files use Prettier code style!
```

Exit status: `0`

## Tauri permission and build validation

Command:

```powershell
cargo check --manifest-path src-tauri/Cargo.toml
```

Literal output:

```text
Compiling nte-dps-tool-tauri v0.3.6 (D:\NTE_DPS_TOOL\src-tauri)
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.80s
```

Exit status: `0`

Command:

```powershell
pnpm --dir frontend tauri:build -- --debug --no-bundle
```

Literal completion output:

```text
vite v8.1.5 building client environment for production...
✓ built in 969ms
Compiling nte-dps-tool-tauri v0.3.6 (D:\NTE_DPS_TOOL\src-tauri)
Finished `dev` profile [unoptimized + debuginfo] target(s) in 20.85s
Built application at: D:\NTE_DPS_TOOL\src-tauri\target\debug\nte-dps-tool-tauri.exe
```

Exit status: `0`

Non-blocking build notices: Vite reported existing chunks above 500 kB; the Windows linker reported the generated import-library path as an informational warning.

## Modified hashes

```text
24A11FF374559602421C27C3307A3187816C5FBF738AC49974B9C333E26890F7  frontend/src/components/nte/desktop-titlebar.tsx
51C637416C4D98D3A897A55ACF8D4DE61B45F4683A3441235F58150990B4C30A  frontend/src/lib/tauri/desktop-window-client.ts
D6126D8E30ED4FF50ACE5CF2CFDF2DA8C43E49C61E21E725A064A60DA877F871  frontend/src/lib/tauri/desktop-window-client.test.ts
0219DA7AFAACFB6EE17CD8FC0EEB21FB7EE9917F2E3AE44E6BF2E3C13339D168  frontend/src/features/main-dps/main-dps-page.tsx
963F28DF4E4F35DF1374D339590F03FEA8E7528FB2674825DAE50358197F538A  frontend/src/main.tsx
EB29A732CAC986BF4782362FC9CA263746CB49000A43E4D7FFD6D094469CDE01  frontend/src/lib/browser-context-menu.ts
9594E53D606BEA5C9A26F888CD000D6D17FEFF4CF04AE799756C98EBFB6E7EE3  frontend/src/lib/browser-context-menu.test.ts
D0AA4CE21CF92FFF1EE1D1C6E674CD7ABDC0E2A7749B337D5EE66F5DE2719591  src-tauri/capabilities/desktop-window-topmost.json
```

## Artifact replay

### Modified archive

The ZIP was reopened, all 8 expected entries were extracted, and every extracted SHA-256 matched `modified-manifest.json`.

Literal output:

```text
zip_entries=8
zip_hashes=verified
```

Exit status: `0`

### Patch

The patch was checked and applied to a baseline copy inside an isolated Git repository with `core.autocrlf=false`; all resulting hashes matched the modified working tree.

Literal output:

```text
patch_check=passed
patch_apply=passed
patch_hashes=verified
```

Exit status: `0`

### Rollback

Command:

```powershell
& .codex-artifacts/window-chrome-phase6-20260802/rollback.ps1 -Root .codex-artifacts/window-chrome-phase6-20260802/verification/rollback-replay
```

Literal completion output:

```text
restored frontend/src/components/nte/desktop-titlebar.tsx 7125F77F1BAF132650EB99F89DA49084FFE4AF2C307A6858D2D2AA3ACD9B718A
restored frontend/src/lib/tauri/desktop-window-client.ts 15D608D1D1A96A44F5834BEB4C60DA85437A9B2E8D98EA1DBF7836D609D51930
restored frontend/src/lib/tauri/desktop-window-client.test.ts A96DBCCA0130FBAC676B7E32F8FC2965BD6D32A2BDD028C37F8F31D37E49D091
restored frontend/src/features/main-dps/main-dps-page.tsx 5A2B93D329890BE59BB0556BE3D9B62FC6AF528C705D56DCB5082ED02F6E70E4
restored frontend/src/main.tsx 8BF3C092F7FCB53DD4A92A890D3195A51A5DFA1583401FF7A932A877CF8B3860
removed frontend/src/lib/browser-context-menu.ts
removed frontend/src/lib/browser-context-menu.test.ts
removed src-tauri/capabilities/desktop-window-topmost.json
rollback verified root=D:\NTE_DPS_TOOL\.codex-artifacts\window-chrome-phase6-20260802\verification\rollback-replay
```

Exit status: `0`

The live working-tree hashes were rechecked after the rollback replay and still matched `modified-manifest.json`.
