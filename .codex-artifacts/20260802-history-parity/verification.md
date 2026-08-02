# History parity verification

## Inputs

- Scoped files: `D:\NTE_DPS_TOOL\.codex-artifacts\20260802-history-parity\scoped-files.txt` (13 files)
- Baseline snapshot: `D:\NTE_DPS_TOOL\.codex-artifacts\20260802-history-parity\original`
- Modified snapshot: `D:\NTE_DPS_TOOL\.codex-artifacts\20260802-history-parity\modified-snapshot`

## Baseline behavior

Command: `python D:\NTE_DPS_TOOL\.codex-artifacts\20260802-history-parity\behavior_probe.py D:\NTE_DPS_TOOL\.codex-artifacts\20260802-history-parity\original`

Exit status: `0`

Literal output:

```json
{
  "browser_file_input": true,
  "browser_blob_export": true,
  "native_import_command": false,
  "native_export_command": false,
  "capture_json_enabled": false,
  "history_context_menu": false,
  "separate_compare_warnings": false,
  "neutral_delta": false,
  "display_row_limit_6": false
}
```

## Modified behavior

Command: `python D:\NTE_DPS_TOOL\.codex-artifacts\20260802-history-parity\behavior_probe.py D:\NTE_DPS_TOOL\.codex-artifacts\20260802-history-parity\modified-snapshot`

Exit status: `0`

Literal output:

```json
{
  "browser_file_input": false,
  "browser_blob_export": false,
  "native_import_command": true,
  "native_export_command": true,
  "capture_json_enabled": true,
  "history_context_menu": true,
  "separate_compare_warnings": true,
  "neutral_delta": true,
  "display_row_limit_6": true
}
```

## Automated validation

| Command | Exit status | Literal result / full log |
| --- | ---: | --- |
| `cargo fmt --check` | `0` | `finished successfully`; `.codex-artifacts/20260802-history-parity/validation-logs/root-fmt.log` |
| `cargo check` | `0` | `finished successfully`; `.codex-artifacts/20260802-history-parity/validation-logs/root-check.log` |
| `cargo test` | `0` | `658 passed, 0 failed, 6 ignored`; `.codex-artifacts/20260802-history-parity/validation-logs/root-test.log` |
| `cargo check --bin nte-dps-tool --features gui` | `0` | `finished successfully`; `.codex-artifacts/20260802-history-parity/validation-logs/gui-check.log` |
| `cargo test --features gui` | `0` | `658 passed, 0 failed, 6 ignored`; `.codex-artifacts/20260802-history-parity/validation-logs/gui-test.log` |
| `cargo clippy --bin nte-dps-tool --features gui -- -D warnings` | `0` | `finished successfully with -D warnings`; `.codex-artifacts/20260802-history-parity/validation-logs/gui-clippy.log` |
| `cargo check --bin nte-core --no-default-features --features cli` | `0` | `finished successfully`; `.codex-artifacts/20260802-history-parity/validation-logs/cli-check.log` |
| `cargo test --no-default-features --features cli` | `0` | `427 passed, 0 failed, 6 ignored`; `.codex-artifacts/20260802-history-parity/validation-logs/cli-test.log` |
| `cargo clippy --bin nte-core --no-default-features --features cli -- -D warnings` | `0` | `finished successfully with -D warnings`; `.codex-artifacts/20260802-history-parity/validation-logs/cli-clippy.log` |
| `cargo fmt --check --manifest-path src-tauri/Cargo.toml` | `0` | `finished successfully`; `.codex-artifacts/20260802-history-parity/validation-logs/tauri-fmt.log` |
| `cargo check --manifest-path src-tauri/Cargo.toml` | `0` | `finished successfully`; `.codex-artifacts/20260802-history-parity/validation-logs/tauri-check.log` |
| `cargo test --manifest-path src-tauri/Cargo.toml` | `0` | `112 passed, 0 failed`; `.codex-artifacts/20260802-history-parity/validation-logs/tauri-test.log` |
| `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` | `1` | `4 pre-existing errors outside scoped history files`; `.codex-artifacts/20260802-history-parity/validation-logs/tauri-clippy.log` |
| `pnpm --dir frontend typecheck` | `0` | `tsc -b completed`; `.codex-artifacts/20260802-history-parity/validation-logs/frontend-typecheck.log` |
| `pnpm --dir frontend lint` | `0` | `oxlint completed with 0 warnings and 0 errors`; `.codex-artifacts/20260802-history-parity/validation-logs/frontend-lint.log` |
| `pnpm --dir frontend test` | `0` | `62 files passed; 212 tests passed`; `.codex-artifacts/20260802-history-parity/validation-logs/frontend-test.log` |
| `pnpm --dir frontend build` | `0` | `Vite built 3371 modules`; `.codex-artifacts/20260802-history-parity/validation-logs/frontend-build.log` |
| `pnpm --dir frontend exec prettier --check src/features/history/history-page.tsx src/features/history/use-history.ts src/features/history/history-view-model.ts src/features/history/history-view-model.test.ts src/lib/tauri/history-client.ts src/lib/tauri/history-client.test.ts src/lib/tauri/history-contract.ts src/lib/tauri/history-contract.test.ts` | `0` | `all matched files use Prettier formatting`; `.codex-artifacts/20260802-history-parity/validation-logs/history-prettier.log` |
| `cargo test japanese_locale_covers_every_simplified_chinese_key` | `0` | `1 passed, 0 failed`; `.codex-artifacts/20260802-history-parity/validation-logs/i18n-parity.log` |
| `git diff --check` | `0` | `no whitespace errors`; `.codex-artifacts/20260802-history-parity/validation-logs/diff-check.log` |

The standalone Tauri Clippy result is nonzero only for:

- `src-tauri/src/commands/empty_curtain.rs:402`
- `src-tauri/src/commands/empty_curtain.rs:429`
- `src-tauri/src/commands/settings.rs:431`
- `src-tauri/src/contract/main_dps_detail.rs:101`

No scoped history file appears in that diagnostic.

CLI dependency command: `cargo tree -e normal --no-default-features --features cli`

Literal result: `FORBIDDEN_GUI_DEPS=0` (tree stored in `cli-tree.txt`).

## Patch application

Command: `git -C D:\NTE_DPS_TOOL\.codex-artifacts\20260802-history-parity\validation\patch-apply-root apply --check --ignore-space-change --ignore-whitespace --whitespace=nowarn D:\NTE_DPS_TOOL\.codex-artifacts\20260802-history-parity\history-parity.patch`

Exit status: `0`

Command: `git -C D:\NTE_DPS_TOOL\.codex-artifacts\20260802-history-parity\validation\patch-apply-root apply --ignore-space-change --ignore-whitespace --whitespace=nowarn D:\NTE_DPS_TOOL\.codex-artifacts\20260802-history-parity\history-parity.patch`

Exit status: `0`

Literal output: `patch_logical_verified_files=13`

## Rollback

Command: `& D:\NTE_DPS_TOOL\.codex-artifacts\20260802-history-parity\rollback.ps1 -Root D:\NTE_DPS_TOOL\.codex-artifacts\20260802-history-parity\validation\rollback-root`

Exit status: `0`

Literal output: `rollback_verified_files=13`

Command: `python D:\NTE_DPS_TOOL\.codex-artifacts\20260802-history-parity\verify_hashes.py D:\NTE_DPS_TOOL\.codex-artifacts\20260802-history-parity\validation\rollback-root D:\NTE_DPS_TOOL\.codex-artifacts\20260802-history-parity\original-hashes.json`

Exit status: `0`

Literal output: `verified_files=13`

## Workspace artifact integrity

Command: `python D:\NTE_DPS_TOOL\.codex-artifacts\20260802-history-parity\verify_hashes.py D:\NTE_DPS_TOOL D:\NTE_DPS_TOOL\.codex-artifacts\20260802-history-parity\modified-hashes.json`

Exit status: `0`

Literal output: `verified_files=13`
