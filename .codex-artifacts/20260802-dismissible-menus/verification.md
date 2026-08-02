# Dismissible menu verification

## Baseline

Audit command:

```powershell
Get-ChildItem frontend/src -Recurse -File -Include *.tsx | Select-String -Pattern 'aria-expanded|columnsOpen|replayOpen|appearanceOpen'
```

Confirmed inputs and behavior:

```text
Main DPS Appearance: trigger toggled appearanceOpen; no outside pointer handler
Main DPS Import Replay: trigger toggled replayOpen; no outside pointer handler
Combat Details Column settings: trigger toggled columnsOpen; no outside pointer handler
HUD modules: trigger toggled open; no outside pointer handler
```

History and Timeline context menus, Ctrl+K palette, and Empty Curtain native dialogs already had outside-dismiss behavior and were retained.

Exit status: `0`

## Modified behavior

All four click-triggered menus use `useDismissibleLayer`. A pointer press outside both trigger and layer closes the menu while presses inside remain interactive. Escape, window blur, and captured scroll also close it. Listeners are removed whenever the menu closes or its component unmounts.

Focused command:

```powershell
pnpm test -- src/hooks/use-dismissible-layer.test.ts src/features/main-dps/main-dps-model.test.ts
```

Working directory: `D:\NTE_DPS_TOOL\frontend`

Literal output:

```text
Test Files  2 passed (2)
Tests  7 passed (7)
```

Exit status: `0`

Full frontend commands:

```powershell
pnpm typecheck
pnpm lint
pnpm test
pnpm build
```

Literal results:

```text
typecheck: exit 0
lint: exit 0
Test Files  65 passed (65)
Tests  222 passed (222)
build: 3373 modules transformed; built in 1.30s; exit 0
```

The production build retained the existing large-chunk advisory.

Formatting and whitespace commands:

```powershell
pnpm exec prettier --check src/hooks/use-dismissible-layer.ts src/hooks/use-dismissible-layer.test.ts src/features/main-dps/main-dps-page.tsx src/features/main-dps/main-dps-detail-page.tsx src/features/technical-hud/technical-hud-page.tsx
git diff --check -- frontend/src/features/main-dps/main-dps-page.tsx frontend/src/features/main-dps/main-dps-detail-page.tsx frontend/src/features/technical-hud/technical-hud-page.tsx
```

Literal results:

```text
All matched files use Prettier code style!
FORMAT_EXIT=0 DIFF_CHECK_EXIT=0
```

Exit statuses: `0`, `0`

## Patch and rollback

Patch commands:

```powershell
git -C <temporary-original-copy> apply --check changes.patch
git -C <temporary-original-copy> apply changes.patch
```

Literal results:

```text
PATCH_CHECK_EXIT=0 PATCH_APPLY_EXIT=0
PATCH_NORMALIZED_TEXT_MISMATCHES=0
MODIFIED_CURRENT_HASH_MISMATCHES=0
```

Rollback command:

```powershell
./rollback.ps1 -TargetRoot <temporary-modified-copy>
```

Literal results:

```text
ROLLBACK_VERIFIED restored=3 removed=2
ROLLBACK_HASH_MISMATCHES=0
ROLLBACK_NEW_FILES_REMAINING=0
ROLLBACK_EXIT=0
```

Temporary verification roots were removed after these checks.
