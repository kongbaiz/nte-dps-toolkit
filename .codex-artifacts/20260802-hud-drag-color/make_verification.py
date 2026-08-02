from pathlib import Path
import os, subprocess

root=Path(r'D:\NTE_DPS_TOOL')
artifact=root/'.codex-artifacts/20260802-hud-drag-color'
env=os.environ.copy(); env['NO_COLOR']='1'; env['CARGO_TERM_COLOR']='never'
def run(command):
    result=subprocess.run(command,cwd=root,env=env,shell=True,text=True,capture_output=True,encoding='utf-8',errors='replace')
    output=(result.stdout+result.stderr).strip()
    return result.returncode,output
baseline_code,baseline=run(r'python .codex-artifacts\20260802-hud-drag-color\behavior_probe.py --tree baseline')
modified_code,modified=run(r'python .codex-artifacts\20260802-hud-drag-color\behavior_probe.py --tree modified')
frontend_code,frontend=run(r'pnpm --dir frontend test -- src/hooks/use-hud-module-pointer-reorder.test.ts src/features/main-dps/main-dps-model.test.ts')
rust_code,rust=run(r'cargo test --features gui character_colors_are_unique_and_independent_of_hashmap_order')
patch_code,patch=run(r'git apply --check --reverse --ignore-space-change --ignore-whitespace .codex-artifacts\20260802-hud-drag-color\change.patch')
record=f'''# HUD drag, window mutuality, and character color verification

## Baseline probe

Command:
```powershell
python .codex-artifacts/20260802-hud-drag-color/behavior_probe.py --tree baseline
```
Input: `.codex-artifacts/20260802-hud-drag-color/original`
Output:
```text
{baseline}
```
Exit status: `{baseline_code}`

## Modified probe

Command:
```powershell
python .codex-artifacts/20260802-hud-drag-color/behavior_probe.py --tree modified
```
Input: `.codex-artifacts/20260802-hud-drag-color/modified`
Output:
```text
{modified}
```
Exit status: `{modified_code}`

## Pointer and accent regression tests

Command:
```powershell
pnpm --dir frontend test -- src/hooks/use-hud-module-pointer-reorder.test.ts src/features/main-dps/main-dps-model.test.ts
```
Output:
```text
{frontend}
```
Exit status: `{frontend_code}`

## Catalog-wide unique color regression

Command:
```powershell
cargo test --features gui character_colors_are_unique_and_independent_of_hashmap_order
```
Output:
```text
{rust}
```
Exit status: `{rust_code}`

## Patch verification

Command:
```powershell
git apply --check --reverse --ignore-space-change --ignore-whitespace .codex-artifacts/20260802-hud-drag-color/change.patch
```
Output:
```text
{patch}
```
Exit status: `{patch_code}`

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
'''
(artifact/'verification.md').write_text(record,encoding='utf-8')
print(f'BASELINE_PROBE_EXIT={baseline_code}')
print(f'MODIFIED_PROBE_EXIT={modified_code}')
print(f'FRONTEND_FOCUSED_EXIT={frontend_code}')
print(f'RUST_FOCUSED_EXIT={rust_code}')
print(f'PATCH_CHECK_EXIT={patch_code}')
if any((baseline_code,modified_code,frontend_code,rust_code,patch_code)): raise SystemExit(1)



