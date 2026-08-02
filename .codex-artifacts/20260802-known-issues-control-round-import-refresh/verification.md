# Verification Record

## Source baseline -> modified behavior check

Command:

```powershell
@'<python source assertions>'@ | python -
```

Inputs:
- `original/` preserved pre-change source
- `modified/` post-change source

Literal output:

```text
BASELINE_THEME_UNAVAILABLE=true
MODIFIED_THEME_TOGGLE=true
BASELINE_TIMELINE_LIVE_CAPTURE=true
MODIFIED_TIMELINE_SELECTED_ROUND=true
BASELINE_IMPORT_RESERVATION=false
MODIFIED_IMPORT_RESERVATION=true
BASELINE_REFRESH_REVISION=false
MODIFIED_REFRESH_REVISION=true
SOURCE_BEHAVIOR_CHECK_EXIT=0
```

Exit status: `0`

## Tauri no-bundle application build

Baseline command:

```powershell
.\frontend\node_modules\.bin\tauri.cmd build --debug --no-bundle
```

Literal result: frontend production build completed; the Rust link/copy step reported `Access is denied. (os error 5)` for the existing `src-tauri\target\debug\nte-dps-tool-tauri.exe`.

Exit status: `1`

Corrected isolated-target command:

```powershell
$env:CARGO_TARGET_DIR='D:\NTE_DPS_TOOL\.codex-artifacts\20260802-known-issues-control-round-import-refresh\tauri-build-target'
.\frontend\node_modules\.bin\tauri.cmd build --debug --no-bundle
```

Literal final output:

```text
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1m 08s
Built application at: D:\NTE_DPS_TOOL\.codex-artifacts\20260802-known-issues-control-round-import-refresh\tauri-build-target\debug\nte-dps-tool-tauri.exe
```

Exit status: `0`

The isolated target directory was removed after verification; the verified build output is recorded above and the source artifact, patch, and rollback remain reproducible.

## Focused frontend regression

Command:

```powershell
pnpm test -- console-command-palette-model.test.ts console-control-client.test.ts settings-client.test.ts settings-contract.test.ts
pnpm typecheck
```

Literal output:

```text
Test Files  4 passed (4)
Tests  19 passed (19)
FOCUSED_TEST_EXIT=0
TYPECHECK_EXIT=0
```

Exit status: `0`

## Full frontend verification

Commands and literal results:

```text
pnpm lint       -> LINT=0
pnpm test       -> Test Files 64 passed (64); Tests 219 passed (219); TEST=0
pnpm build      -> built in 901ms; BUILD=0
pnpm format:check -> FORMAT=1; 31 pre-existing files outside this task reported
pnpm exec prettier --check <8 task frontend files> -> All matched files use Prettier code style; exit 0
```

The production build emitted only the existing chunk-size advisory.

## Tauri verification

Commands:

```powershell
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
```

Literal output:

```text
cargo fmt --check: exit 0
cargo check: Finished dev profile; exit 0
cargo test: 117 passed; 0 failed; exit 0
cargo clippy: Finished dev profile; exit 0
```

## Patch verification

Commands:

```powershell
git -C <temporary-original-copy> apply --check changes.patch
git -C <temporary-original-copy> apply changes.patch
```

Literal output:

```text
PATCH_CHECK_EXIT=0 PATCH_APPLY_EXIT=0
PATCH_NORMALIZED_TEXT_MISMATCHES=0
```

Exit status: `0`

## Rollback verification

Command:

```powershell
./rollback.ps1 -TargetRoot <temporary-modified-copy>
```

Literal output:

```text
ROLLBACK_VERIFIED files=15 removed=3
ROLLBACK_EXECUTED_EXIT=0 NEW_FILES_REMAINING=0
```

Exit status: `0`

## Whitespace verification

Command:

```powershell
git diff --check -- <task tracked files>
```

Literal output: no whitespace errors; only Git line-ending notices.

Exit status: `0`
