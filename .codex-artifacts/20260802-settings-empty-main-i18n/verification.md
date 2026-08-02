# Settings / Empty Curtain / Main Window / i18n verification

## Scope and roles

- Original snapshot: `D:\NTE_DPS_TOOL\.codex-artifacts\20260802-settings-empty-main-i18n\original` with `D:\NTE_DPS_TOOL\.codex-artifacts\20260802-settings-empty-main-i18n\original-hashes.json`.
- Modified artifact: `D:\NTE_DPS_TOOL\.codex-artifacts\20260802-settings-empty-main-i18n\modified-snapshot` with `D:\NTE_DPS_TOOL\.codex-artifacts\20260802-settings-empty-main-i18n\modified-hashes.json`.
- Patch/diff: `D:\NTE_DPS_TOOL\.codex-artifacts\20260802-settings-empty-main-i18n\parity-fixes.patch`.
- Verification record: `D:\NTE_DPS_TOOL\.codex-artifacts\20260802-settings-empty-main-i18n\verification.md` plus `validation-results.json` and `validation-logs/`.
- Runnable rollback: `D:\NTE_DPS_TOOL\.codex-artifacts\20260802-settings-empty-main-i18n\rollback.ps1`.
- Changed fields/branches: native Settings team-data import and NIC states; Empty Curtain equipment pointer intent; main-window capture/list/context-menu state; zh-CN/ja locale keys.

## Baseline behavior

Command:

```powershell
python .codex-artifacts/20260802-settings-empty-main-i18n/behavior_probe.py .codex-artifacts/20260802-settings-empty-main-i18n/original
```

Input: the byte-preserved `original/` snapshot listed by `scoped-files.txt` and hashed by `original-hashes.json`.

Literal output:

```json
{
  "settings_browser_file_input": true,
  "settings_native_import_command": false,
  "settings_two_column_breakpoint_900": false,
  "settings_nic_empty_warning": false,
  "settings_standalone_title": true,
  "empty_standalone_header": true,
  "empty_right_click_management": false,
  "empty_exact_instruction": false,
  "main_context_detail_action": false,
  "main_unattributed_state": false,
  "main_capture_status_tone": false,
  "main_redetect_action": false,
  "remaining_i18n_keys_present": false,
  "scoped_tauri_clippy_patterns_fixed": false
}
```

Exit status: `0`.

## Modified behavior

Command:

```powershell
python .codex-artifacts/20260802-settings-empty-main-i18n/behavior_probe.py .codex-artifacts/20260802-settings-empty-main-i18n/modified-snapshot
```

Input: the byte-preserved `modified-snapshot/` hashed by `modified-hashes.json`.

Literal output:

```json
{
  "settings_browser_file_input": false,
  "settings_native_import_command": true,
  "settings_two_column_breakpoint_900": true,
  "settings_nic_empty_warning": true,
  "settings_standalone_title": false,
  "empty_standalone_header": false,
  "empty_right_click_management": true,
  "empty_exact_instruction": true,
  "main_context_detail_action": true,
  "main_unattributed_state": true,
  "main_capture_status_tone": true,
  "main_redetect_action": true,
  "remaining_i18n_keys_present": true,
  "scoped_tauri_clippy_patterns_fixed": true
}
```

Exit status: `0`.

## Automated validation matrix

| Check | Exact command | Exit | Log |
| --- | --- | ---: | --- |
| `root-fmt` | `cargo fmt --check` | 0 | `validation-logs/root-fmt.log` |
| `root-check` | `cargo check` | 0 | `validation-logs/root-check.log` |
| `root-test` | `cargo test` | 0 | `validation-logs/root-test.log` |
| `gui-check` | `cargo check --bin nte-dps-tool --features gui` | 0 | `validation-logs/gui-check.log` |
| `gui-test` | `cargo test --features gui` | 0 | `validation-logs/gui-test.log` |
| `gui-clippy` | `cargo clippy --bin nte-dps-tool --features gui -- -D warnings` | 0 | `validation-logs/gui-clippy.log` |
| `cli-check` | `cargo check --bin nte-core --no-default-features --features cli` | 0 | `validation-logs/cli-check.log` |
| `cli-test` | `cargo test --no-default-features --features cli` | 0 | `validation-logs/cli-test.log` |
| `cli-clippy` | `cargo clippy --bin nte-core --no-default-features --features cli -- -D warnings` | 0 | `validation-logs/cli-clippy.log` |
| `cli-tree` | `cargo tree -e normal --no-default-features --features cli` | 0 | `validation-logs/cli-tree.log` |
| `tauri-fmt` | `cargo fmt --check --manifest-path src-tauri/Cargo.toml` | 0 | `validation-logs/tauri-fmt.log` |
| `tauri-check` | `cargo check --manifest-path src-tauri/Cargo.toml` | 0 | `validation-logs/tauri-check.log` |
| `tauri-focused-test` | `cargo test --manifest-path src-tauri/Cargo.toml settings` | 0 | `validation-logs/tauri-focused-test.log` |
| `tauri-test` | `cargo test --manifest-path src-tauri/Cargo.toml` | 0 | `validation-logs/tauri-test.log` |
| `tauri-clippy` | `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` | 0 | `validation-logs/tauri-clippy.log` |
| `frontend-focused-test` | `pnpm --dir frontend test -- src/lib/tauri/settings-client.test.ts src/lib/tauri/settings-contract.test.ts src/features/empty-curtain/equipment-canvas-layout.test.ts src/features/main-dps/main-dps-model.test.ts src/lib/i18n.test.ts` | 0 | `validation-logs/frontend-focused-test.log` |
| `frontend-typecheck` | `pnpm --dir frontend typecheck` | 0 | `validation-logs/frontend-typecheck.log` |
| `frontend-lint` | `pnpm --dir frontend lint` | 0 | `validation-logs/frontend-lint.log` |
| `frontend-test` | `pnpm --dir frontend test` | 0 | `validation-logs/frontend-test.log` |
| `frontend-build` | `pnpm --dir frontend build` | 0 | `validation-logs/frontend-build.log` |
| `frontend-prettier` | `pnpm --dir frontend exec prettier --check src/features/settings/settings-page.tsx src/features/settings/settings-catalog.tsx src/features/settings/use-settings.ts src/lib/tauri/settings-client.ts src/lib/tauri/settings-client.test.ts src/lib/tauri/settings-contract.ts src/lib/tauri/settings-contract.test.ts src/features/empty-curtain/empty-curtain-page.tsx src/features/empty-curtain/equipment-canvas-grid.tsx src/features/empty-curtain/equipment-canvas-layout.ts src/features/empty-curtain/equipment-canvas-layout.test.ts src/features/main-dps/main-dps-page.tsx src/features/main-dps/main-dps-model.ts src/features/main-dps/main-dps-model.test.ts src/lib/i18n.test.ts` | 0 | `validation-logs/frontend-prettier.log` |
| `i18n-audit` | `python .codex-artifacts/20260802-settings-empty-main-i18n/audit_production_i18n.py` | 0 | `validation-logs/i18n-audit.log` |
| `diff-check` | `git diff --check` | 0 | `validation-logs/diff-check.log` |
| `cli-tree-audit` | `inspect cli-tree.log for tauri/wry/webview2-com/eframe/egui/wgpu/rfd/raw-window-handle` | 0 | `validation-logs/cli-tree-audit.log` |
| `tauri-debug-build` | `.\frontend\node_modules\.bin\tauri.cmd build --debug --no-bundle` | 0 | `validation-logs/tauri-debug-build.log` |

All 25 checks exited `0`. Selected literal results:

- root: `651 passed; 0 failed; 7 ignored`
- CLI: `427 passed; 0 failed; 6 ignored`
- Tauri focused Settings: `14 passed; 0 failed`
- Tauri full: `113 passed; 0 failed`
- frontend focused: `5 passed` files / `30 passed` tests
- frontend full: `63 passed` files / `218 passed` tests
- Tauri debug/no-bundle integration build: application built at `src-tauri/target/debug/nte-dps-tool-tauri.exe`
- production i18n: `production_missing_zh=[]`, `production_missing_ja=[]`, `locale_key_delta=0`
- CLI-only dependency audit: `FORBIDDEN_GUI_DEPS=0`
- focused Prettier: `All matched files use Prettier code style!`

## Patch, artifact, and rollback execution

The patch applies through Git and is text-equivalent across all 24 scoped files. Git normalizes text line endings in the isolated patch root, so byte-exact verification is owned by `modified-snapshot/` and `modified-hashes.json`. Rollback restores byte-exact original hashes and removes the file that was absent at baseline.

Literal delivery log:

```text
COMMAND: git apply --check --ignore-space-change --ignore-whitespace --whitespace=nowarn parity-fixes.patch
OUTPUT: patch_check=passed
EXIT_STATUS: 0
COMMAND: git apply --ignore-space-change --ignore-whitespace --whitespace=nowarn parity-fixes.patch
OUTPUT: patch_apply=passed
EXIT_STATUS: 0
COMMAND: python verify_text_equivalence.py PATCH_ROOT modified-snapshot scoped-files.txt
text_equivalent_files=24
EXIT_STATUS: 0
COMMAND: python behavior_probe.py PATCH_ROOT
{
  "settings_browser_file_input": false,
  "settings_native_import_command": true,
  "settings_two_column_breakpoint_900": true,
  "settings_nic_empty_warning": true,
  "settings_standalone_title": false,
  "empty_standalone_header": false,
  "empty_right_click_management": true,
  "empty_exact_instruction": true,
  "main_context_detail_action": true,
  "main_unattributed_state": true,
  "main_capture_status_tone": true,
  "main_redetect_action": true,
  "remaining_i18n_keys_present": true,
  "scoped_tauri_clippy_patterns_fixed": true
}
EXIT_STATUS: 0
COMMAND: powershell -File rollback.ps1 -Root ROLLBACK_ROOT
rollback_verified_files=24
EXIT_STATUS: 0
OUTPUT: rollback_reopened_verified_files=24
COMMAND: python verify_hashes.py WORKSPACE modified-hashes.json
verified_files=24
EXIT_STATUS: 0
COMMAND: reopen original-hashes.json modified-hashes.json parity-fixes.patch verification inputs rollback.ps1
OUTPUT: delivery_roles_reopened=6
EXIT_STATUS: 0
VALIDATION_ROOT: D:\NTE_DPS_TOOL\.codex-artifacts\20260802-settings-empty-main-i18n\delivery-validation-20260802-105744207
```

## Rollback command

```powershell
& 'D:\NTE_DPS_TOOL\.codex-artifacts\20260802-settings-empty-main-i18n\rollback.ps1' -Root 'D:\NTE_DPS_TOOL'
```

The rollback guard first verifies every modified hash, then restores the original snapshot, removes original-absent additions, and verifies all original states.
