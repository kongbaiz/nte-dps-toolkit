# Main DPS appearance popover verification

## Baseline

Command:

```powershell
Select-String -Path frontend/src/index.css -Pattern 'main-dps-toolbar|overflow-x' -Context 0,4
Select-String -Path frontend/src/features/main-dps/main-dps-page.tsx -Pattern 'absolute right-0 top-9'
```

Input: main window at or below the `max-height: 560px` compact breakpoint.

Literal relevant output:

```text
.main-dps-toolbar {
  flex-wrap: nowrap;
  justify-content: flex-start;
  overflow-x: auto;
}
<div className="absolute right-0 top-9 ...">
```

Confirmed behavior: the appearance panel was an absolutely positioned child of the compact toolbar scroll container, so the toolbar clipped the panel outside its own height.

Exit status: `0`

## Modified behavior

The appearance panel is rendered through `createPortal(document.body)` with fixed viewport coordinates. `appearancePanelPosition` clamps the panel to an 8 px viewport margin, opens above the trigger when lower space is insufficient, and recalculates on window resize or captured scroll.

Focused command:

```powershell
pnpm test -- src/features/main-dps/main-dps-model.test.ts
```

Working directory: `D:\NTE_DPS_TOOL\frontend`

Literal output:

```text
Test Files  1 passed (1)
Tests  5 passed (5)
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
Test Files  64 passed (64)
Tests  220 passed (220)
build: 3372 modules transformed; built in 1.14s; exit 0
```

The build retained the existing large-chunk advisory.

Formatting and whitespace commands:

```powershell
pnpm exec prettier --check src/features/main-dps/main-dps-page.tsx src/features/main-dps/main-dps-model.ts src/features/main-dps/main-dps-model.test.ts
git diff --check -- frontend/src/features/main-dps/main-dps-page.tsx frontend/src/features/main-dps/main-dps-model.ts frontend/src/features/main-dps/main-dps-model.test.ts
```

Literal results:

```text
All matched files use Prettier code style!
DIFF_CHECK_EXIT=0
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

Literal result:

```text
ROLLBACK_VERIFIED files=3
ROLLBACK_HASH_MISMATCHES=0
ROLLBACK_EXIT=0
```

Temporary verification roots were removed after these checks.
