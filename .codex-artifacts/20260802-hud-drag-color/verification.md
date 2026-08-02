# HUD drag, window mutuality, and character color verification

## Baseline probe

Command:
```powershell
python .codex-artifacts/20260802-hud-drag-color/behavior_probe.py --tree baseline
```
Input: `.codex-artifacts/20260802-hud-drag-color/original`
Output:
```text
TREE=baseline
HUD_NATIVE_DRAG=true
SETTINGS_NATIVE_DRAG=true
SETTINGS_TWO_COLUMN=true
SETTINGS_ONE_COLUMN=false
SETTINGS_HIDES_MAIN=false
AVATAR_PIXEL_COLOR_SOURCE=true
DISTINCT_PALETTE_SIZE=0
ASSIGNMENT_USES_ATTRIBUTE=false
```
Exit status: `0`

## Modified probe

Command:
```powershell
python .codex-artifacts/20260802-hud-drag-color/behavior_probe.py --tree modified
```
Input: `.codex-artifacts/20260802-hud-drag-color/modified`
Output:
```text
TREE=modified
HUD_NATIVE_DRAG=false
SETTINGS_NATIVE_DRAG=false
SETTINGS_TWO_COLUMN=false
SETTINGS_ONE_COLUMN=true
SETTINGS_HIDES_MAIN=true
AVATAR_PIXEL_COLOR_SOURCE=false
DISTINCT_PALETTE_SIZE=32
ASSIGNMENT_USES_ATTRIBUTE=false
```
Exit status: `0`

## Pointer and accent regression tests

Command:
```powershell
pnpm --dir frontend test -- src/hooks/use-hud-module-pointer-reorder.test.ts src/features/main-dps/main-dps-model.test.ts
```
Output:
```text
RUN  v4.1.10 D:/NTE_DPS_TOOL/frontend


 Test Files  67 passed (67)
      Tests  227 passed (227)
   Start at  19:23:01
   Duration  1.53s (transform 3.59s, setup 0ms, import 5.65s, tests 455ms, environment 7ms)

$ vitest run "--" "src/hooks/use-hud-module-pointer-reorder.test.ts" "src/features/main-dps/main-dps-model.test.ts"
```
Exit status: `0`

## Catalog-wide unique color regression

Command:
```powershell
cargo test --features gui character_colors_are_unique_and_independent_of_hashmap_order
```
Output:
```text
running 1 test
test storage::resource::tests::character_colors_are_unique_and_independent_of_hashmap_order ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 661 filtered out; finished in 0.00s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.28s
     Running unittests src\lib.rs (target\debug\deps\nte_dps_tool-029cfa580a9f8ca1.exe)
     Running unittests src\main.rs (target\debug\deps\nte_dps_tool-00ddf7cf4f32e695.exe)
     Running unittests src\bin\nte-updater.rs (target\debug\deps\nte_updater-b72d2d3a995e2b0c.exe)
```
Exit status: `0`

## Patch verification

Command:
```powershell
git apply --check --reverse --ignore-space-change --ignore-whitespace .codex-artifacts/20260802-hud-drag-color/change.patch
```
Output:
```text

```
Exit status: `0`

## Rollback smoke verification

Command:
```powershell
.codex-artifacts/20260802-hud-drag-color/rollback.ps1 -WorkspaceRoot .codex-artifacts/20260802-hud-drag-color/rollback-smoke-final
```
Output:
```text
ROLLBACK_RESTORED=13
ROLLBACK_REMOVED=3
FINAL_ROLLBACK_HASH_MISMATCHES=0
FINAL_ROLLBACK_NEW_FILES_REMAINING=0
FINAL_MODIFIED_HASH_MISMATCHES=0
FINAL_FOUR_ROLE_VERIFICATION_EXIT=0
```
Exit status: `0`

## Full validation exit summary

```text
CARGO_FMT_EXIT=0
CARGO_CHECK_EXIT=0
CARGO_TEST_EXIT=0 (655 passed, 7 ignored)
GUI_CHECK_EXIT=0
CLI_CHECK_EXIT=0
GUI_CLIPPY_EXIT=0
CLI_CLIPPY_EXIT=0
GUI_TEST_EXIT=0 (655 passed, 7 ignored)
CLI_TEST_EXIT=0 (429 passed, 6 ignored)
CLI_TREE_FORBIDDEN_COUNT=0
TAURI_FMT_EXIT=0
TAURI_CHECK_EXIT=0
TAURI_TEST_EXIT=0 (120 passed)
TAURI_CLIPPY_EXIT=0
FRONTEND_FORMAT_TARGET_EXIT=0
FRONTEND_LINT_EXIT=0
FRONTEND_TYPECHECK_EXIT=0
FRONTEND_TEST_EXIT=0 (67 files, 227 tests)
FRONTEND_BUILD_EXIT=0
TAURI_DEBUG_NO_BUNDLE_EXIT=0
FINAL_STYLE_AND_DIFF_CHECK_EXIT=0
FRONTEND_FULL_FORMAT_EXIT=1 (22 pre-existing out-of-scope files; no task file listed)
```
